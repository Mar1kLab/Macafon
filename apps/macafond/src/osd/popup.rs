//! Windows popup: a layered, click-through, topmost window owned by its own thread. Direct2D draws
//! each frame into a WIC bitmap, `UpdateLayeredWindow` shows it; springs drive every motion and the
//! loop paces itself on `DwmFlush` while anything moves.
use super::dictation::{State as DictationState, View as DictationView};
use super::icons::ICONS;
use super::motion::*;
use super::{Osd, StatusIcon};
use macafon_config::{OsdConfig, OsdEdge, OsdLook, OsdStyle};
use std::collections::HashMap;
use std::sync::{OnceLock, mpsc};
use std::time::{Duration, Instant};
#[cfg(test)]
use windows::Win32::Foundation::GENERIC_WRITE;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::Win32::UI::WindowsAndMessaging::*;
#[cfg(test)]
use windows::core::HSTRING;
use windows::core::{BOOL, Interface, PCWSTR, Result, w};
use windows_numerics::{Matrix3x2, Vector2};

mod painter;
use painter::Painter;

const WAKE: u32 = WM_APP + 1;
// Design tokens (docs/design/design-system.md).
const BG_RAISED: u32 = 0x101014;
const INK: u32 = 0xf2f1ee;
const WHITE: u32 = 0xffffff;
const ACCENT: u32 = 0x2d81ff;
const DANGER: u32 = 0xff6259;

type Message = (Osd, OsdConfig);

static POPUP: OnceLock<Option<(mpsc::Sender<Message>, usize)>> = OnceLock::new();
thread_local! { static DICTATION_BUTTONS: std::cell::Cell<Option<(windows::Win32::Foundation::RECT, windows::Win32::Foundation::RECT)>> = const { std::cell::Cell::new(None) }; }

pub(super) fn clipboard_owner() -> Option<HWND> {
    POPUP
        .get_or_init(spawn)
        .as_ref()
        .map(|(_, window)| HWND(*window as *mut _))
}

pub fn show(osd: Osd, setup: OsdConfig) {
    let Some((sender, window)) = POPUP.get_or_init(spawn) else {
        return;
    };
    if sender.send((osd, setup.normalized())).is_ok() {
        // SAFETY: posts to the popup thread's window without borrowing its memory.
        let _ = unsafe { PostMessageW(Some(HWND(*window as *mut _)), WAKE, WPARAM(0), LPARAM(0)) };
    }
}

fn spawn() -> Option<(mpsc::Sender<Message>, usize)> {
    let (sender, receiver) = mpsc::channel();
    let (ready, started) = mpsc::sync_channel(1);
    let spawned = std::thread::Builder::new()
        .name("macafon-osd".into())
        .spawn(move || {
            if let Err(error) = run(receiver, &ready) {
                tracing::warn!(%error, "Hotkey popup unavailable");
                let _ = ready.try_send(None);
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "Cannot start hotkey popup thread");
        return None;
    }
    let window = started.recv().ok()??;
    Some((sender, window))
}

unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_MOUSEACTIVATE {
        return LRESULT(MA_NOACTIVATE as isize);
    }
    if message == WM_NCHITTEST {
        let x = lparam.0 as i16 as i32;
        let y = (lparam.0 >> 16) as i16 as i32;
        if let Some((stop, cancel)) = DICTATION_BUTTONS.with(|b| b.get()) {
            let inside = |r: windows::Win32::Foundation::RECT| {
                x >= r.left && x < r.right && y >= r.top && y < r.bottom
            };
            return LRESULT(if inside(stop) || inside(cancel) {
                HTCLIENT as isize
            } else {
                HTTRANSPARENT as isize
            });
        }
    }
    // SAFETY: window-manager arguments; no application pointers are used.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn run(receiver: mpsc::Receiver<Message>, ready: &mpsc::SyncSender<Option<usize>>) -> Result<()> {
    // SAFETY: this thread owns its window, COM apartment and Direct2D objects.
    unsafe {
        // Physical pixels on every monitor: the popup is drawn for the monitor it lands on.
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let instance = GetModuleHandleW(None)?;
        let class = w!("MacafonOsd");
        let descriptor = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        if RegisterClassW(&descriptor) == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let window = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            w!("Macafon"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        let mut popup = Popup::new(window, Painter::new()?);
        let _ = ready.send(Some(window.0 as usize));
        tracing::info!("Hotkey popup ready");
        let mut message = MSG::default();
        let mut moving = false;
        let mut last = Instant::now();
        loop {
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                if message.message == WM_QUIT {
                    return Ok(());
                }
                if message.message == WM_LBUTTONUP {
                    let x = (message.lParam.0 as i16) as f32 / popup.painter.scale;
                    let y = ((message.lParam.0 >> 16) as i16) as f32 / popup.painter.scale;
                    if let Some(Osd::Dictation(view)) = &popup.motion.view {
                        if view.hands_free && view.state == DictationState::Listening {
                            let panel = dictation_size(view);
                            let (bounds, _, _) = popup.layout.place(popup.painter.size, panel);
                            if y >= bounds.top
                                && y <= bounds.top + 28.0
                                && x >= bounds.right - 50.0
                                && x <= bounds.right
                            {
                                super::dictation_click(x > bounds.right - 28.0);
                            }
                        }
                    }
                }
                DispatchMessageW(&message);
            }
            let now = Instant::now();
            let mut dirty = false;
            while let Ok((osd, setup)) = receiver.try_recv() {
                popup.receive(osd, setup, now);
                dirty = true;
            }
            // The first frame of a motion starts from rest, not from the idle wait.
            let dt = if moving {
                (now - last).as_secs_f32().min(1.0 / 30.0)
            } else {
                0.0
            };
            last = now;
            moving = popup.motion.visible && popup.advance(now, dt);
            if popup.motion.visible && (moving || dirty) {
                if let Err(error) = popup.frame() {
                    tracing::debug!(%error, "Hotkey popup frame failed");
                }
            }
            if moving {
                if DwmFlush().is_err() {
                    std::thread::sleep(Duration::from_millis(8));
                }
                continue;
            }
            let timeout = popup.timeout(Instant::now());
            MsgWaitForMultipleObjectsEx(None, timeout, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        }
    }
}

struct Popup {
    window: HWND,
    painter: Painter,
    layout: Layout,
    motion: Motion,
    shown: bool,
    /// Window origin in screen pixels.
    origin: POINT,
}

impl Popup {
    fn new(window: HWND, painter: Painter) -> Self {
        Self {
            window,
            painter,
            layout: Layout {
                setup: OsdConfig::default(),
                pad: MARGIN,
            },
            motion: Motion::new(),
            shown: false,
            origin: POINT::default(),
        }
    }

    fn receive(&mut self, osd: Osd, setup: OsdConfig, now: Instant) {
        if matches!(&osd, Osd::Dictation(v) if v.state == DictationState::Cancelled) {
            self.hide();
            self.motion.view = None;
            return;
        }
        let same = self.motion.continues(&osd);
        // Settings changing the setup moves a visible popup right away, and a hotkey used
        // in an app on another monitor brings it there; dictation updates leave it in place.
        if !self.motion.visible || setup != self.layout.setup || !matches!(osd, Osd::Dictation(_))
        {
            if let Err(error) = self.place(setup, matches!(osd, Osd::Dictation(_))) {
                tracing::debug!(%error, "Cannot place hotkey popup");
                return;
            }
        }
        let clickable = matches!(&osd, Osd::Dictation(view) if view.hands_free && view.state == DictationState::Listening);
        let painter = &self.painter;
        self.motion
            .receive(osd, same, now, animations_enabled(), |title, value| {
                painter.status_width(title, value, setup.look)
            });
        // SAFETY: this thread owns the window; only its click-through style changes.
        unsafe {
            let style = GetWindowLongPtrW(self.window, GWL_EXSTYLE) as u32;
            SetWindowLongPtrW(
                self.window,
                GWL_EXSTYLE,
                if clickable {
                    style & !WS_EX_TRANSPARENT.0
                } else {
                    style | WS_EX_TRANSPARENT.0
                } as isize,
            );
        }
    }

    /// Moves every spring by `dt`; false once all rest.
    fn advance(&mut self, now: Instant, dt: f32) -> bool {
        match self.motion.advance(now, dt, animations_enabled) {
            Step::Moving => true,
            Step::Resting => false,
            Step::Hidden => {
                self.hide();
                false
            }
        }
    }

    /// Wait until the hold ends while shown, forever while hidden.
    fn timeout(&self, now: Instant) -> u32 {
        self.motion.wait(now).map_or(u32::MAX, |left| {
            u32::try_from(left.as_millis()).unwrap_or(u32::MAX).max(1)
        })
    }

    fn hide(&mut self) {
        DICTATION_BUTTONS.with(|b| b.set(None));
        self.motion.hide();
        self.shown = false;
        // SAFETY: the window belongs to this thread.
        unsafe {
            let _ = ShowWindow(self.window, SW_HIDE);
        }
    }

    /// The chosen edge of the work area of the monitor with the foreground window (else the
    /// cursor), moved toward the center by the offset, drawn at that monitor's scale.
    fn place(&mut self, setup: OsdConfig, dictation: bool) -> Result<()> {
        // SAFETY: plain queries into stack structs.
        unsafe {
            let foreground = GetForegroundWindow();
            let monitor = if setup.primary_monitor {
                MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY)
            } else if foreground.is_invalid() {
                let mut cursor = POINT::default();
                GetCursorPos(&mut cursor)?;
                MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST)
            } else {
                MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST)
            };
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return Err(windows::core::Error::from_thread());
            }
            let (mut dpi_x, mut dpi_y) = (96, 96);
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
            let scale = dpi_x as f32 / 96.0;
            let work = info.rcWork;
            let (work_w, work_h) = (work.right - work.left, work.bottom - work.top);
            let across = match setup.edge {
                OsdEdge::Left | OsdEdge::Right => work_w,
                OsdEdge::Top | OsdEdge::Bottom => work_h,
            };
            let offset = (across as f32 * setup.offset_pct / 100.0).round() as i32;
            let margin = edge_margin(dictation);
            let layout = Layout::new(setup, margin, offset as f32 / scale);
            self.painter.resize(scale, layout.window())?;
            let px = |dip: f32| (dip * scale).round() as i32;
            let (w, h) = (self.painter.width as i32, self.painter.height as i32);
            // The panel's edge lands `MARGIN + offset` from the work area's edge.
            let inset = offset + px(margin) - px(layout.pad);
            self.origin = match setup.edge {
                OsdEdge::Right => POINT {
                    x: work.right - w - inset,
                    y: work.top + (work_h - h) / 2,
                },
                OsdEdge::Left => POINT {
                    x: work.left + inset,
                    y: work.top + (work_h - h) / 2,
                },
                OsdEdge::Bottom => POINT {
                    x: work.left + (work_w - w) / 2,
                    y: work.bottom - h - inset,
                },
                OsdEdge::Top => POINT {
                    x: work.left + (work_w - w) / 2,
                    y: work.top + inset,
                },
            };
            self.layout = layout;
        }
        Ok(())
    }

    fn frame(&mut self) -> Result<()> {
        let Some(scene) = self.motion.scene() else {
            return Ok(());
        };
        self.painter.paint(&scene, self.layout, None)?;
        let buttons = if let Osd::Dictation(view) = scene.view {
            if view.hands_free && view.state == DictationState::Listening {
                let (bounds, _, _) = self.layout.place(self.painter.size, dictation_size(view));
                let p = self.painter.panel;
                let screen = |left: f32, right: f32| windows::Win32::Foundation::RECT {
                    left: self.origin.x + ((left * p.M11 + p.M31) * self.painter.scale) as i32,
                    right: self.origin.x + ((right * p.M11 + p.M31) * self.painter.scale) as i32,
                    top: self.origin.y
                        + (((bounds.top + 1.0) * p.M22 + p.M32) * self.painter.scale) as i32,
                    bottom: self.origin.y
                        + (((bounds.top + 26.0) * p.M22 + p.M32) * self.painter.scale) as i32,
                };
                Some((
                    screen(bounds.right - 48.0, bounds.right - 28.0),
                    screen(bounds.right - 28.0, bounds.right - 7.0),
                ))
            } else {
                None
            }
        } else {
            None
        };
        DICTATION_BUTTONS.with(|b| b.set(buttons));
        let opacity = (self.motion.enter.value * 1.4).clamp(0.0, 1.0);
        self.painter.present(self.window, self.origin, opacity)?;
        if !self.shown {
            self.shown = true;
            // SAFETY: the window belongs to this thread; it never takes focus.
            unsafe {
                let _ = SetWindowPos(
                    self.window,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
        }
        Ok(())
    }
}

/// Windows' "Animation effects" setting (accessibility): off means no motion.
fn animations_enabled() -> bool {
    let mut enabled = BOOL(1);
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL.
    let _ = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(std::ptr::from_mut(&mut enabled).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    enabled.as_bool()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "writes native dictation popup previews"]
    fn dictation_previews() {
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.artifacts/screenshots");
        std::fs::create_dir_all(&directory).expect("preview directory");
        // SAFETY: COM is initialized only for this test's rendering thread.
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .expect("COM");
        }
        let mut painter = Painter::new().expect("painter");
        let layout = Layout {
            setup: OsdConfig {
                style: OsdStyle::Horizontal,
                edge: OsdEdge::Bottom,
                ..Default::default()
            },
            pad: 48.0,
        };
        painter.resize(2.0, layout.window()).expect("surface");
        for (name, look) in [
            ("flow", macafon_ipc::DictationPopupLook::Flow),
            ("analog", macafon_ipc::DictationPopupLook::Analog),
        ] {
            for (label, state) in [
                ("listening", DictationState::Listening),
                ("transcribing", DictationState::Transcribing),
                ("done", DictationState::Done),
                ("copied", DictationState::Copied),
                ("no-speech", DictationState::NoSpeech),
                ("error", DictationState::Error),
                ("too-long", DictationState::Listening),
            ] {
                let view = Osd::Dictation(DictationView {
                    look,
                    state,
                    rms_dbfs: -20.0,
                    bands: std::array::from_fn(|n| 0.3 + (n as f32 * 0.8).sin().abs() * 0.6),
                    elapsed_s: if label == "too-long" { 270 } else { 7 },
                    engine: "WHISPER".into(),
                    words: 14,
                    language: "UK".into(),
                    hands_free: true,
                });
                let scene = Scene {
                    view: &view,
                    enter: 1.0,
                    level: 0.0,
                    mute: 0.0,
                    alert: 0.0,
                    stretch: 0.0,
                    status_width: STATUS_MIN_W,
                    vu: 0.8,
                    vu_peak: 0.9,
                    shimmer: 0.5,
                };
                painter
                    .paint(&scene, layout, Some(BG_RAISED))
                    .expect("paint");
                painter
                    .save_png(&directory.join(format!("2026-10-19-dictation-{name}-{label}.png")))
                    .expect("png");
                painter.resize(1.0, (1180.0, 760.0)).expect("full screen");
                painter
                    .paint(&scene, layout, Some(BG_RAISED))
                    .expect("full frame");
                painter
                    .save_png(
                        &directory.join(format!("2026-10-19-dictation-{name}-{label}-screen.png")),
                    )
                    .expect("full PNG");
                painter.resize(2.0, layout.window()).expect("close up");
            }
        }
    }

    /// `cargo test -p macafond osd_previews -- --ignored` writes PNGs for review.
    #[test]
    #[ignore = "writes preview images"]
    fn osd_previews() {
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.artifacts/screenshots");
        std::fs::create_dir_all(&directory).expect("preview directory");
        // SAFETY: COM for this test thread.
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .expect("COM")
        };
        let mut painter = Painter::new().expect("painter");
        let volume =
            |name: &str, color: &str, icon: &str, volume_pct: f32, muted: bool| Osd::Volume {
                name: name.into(),
                color: color.into(),
                icon: icon.into(),
                volume_pct,
                muted,
            };
        let status = |icon, title: &str, value: &str, alert| Osd::Status {
            icon,
            title: title.into(),
            value: value.into(),
            alert,
        };
        let views = [
            (
                "display",
                status(StatusIcon::Monitor, "Display", "Work", false),
                0.0,
            ),
            ("main", volume("Main", "blue", "speaker", 67.0, false), 0.0),
            (
                "voice-muted",
                volume("Voice", "violet", "message", 40.0, true),
                0.0,
            ),
            (
                "music-overscroll",
                volume("Music", "pink", "music", 100.0, false),
                6.0,
            ),
            (
                "mic-muted",
                status(StatusIcon::MicOff, "Microphone", "Muted", true),
                0.0,
            ),
            (
                "bypass",
                status(StatusIcon::Processing, "Processing", "Bypassed", true),
                0.0,
            ),
            (
                "preset",
                status(StatusIcon::Preset, "Microphone preset", "Natural", false),
                0.0,
            ),
        ];
        let layouts = [
            ("vertical", OsdStyle::Vertical, OsdEdge::Right),
            ("horizontal", OsdStyle::Horizontal, OsdEdge::Bottom),
        ];
        for (look_name, look) in [("classic", OsdLook::Classic), ("sketch", OsdLook::Sketch)] {
            for (style_name, style, edge) in layouts {
                let layout = Layout {
                    setup: OsdConfig {
                        look,
                        primary_monitor: false,
                        style,
                        edge,
                        offset_pct: 0.0,
                    },
                    pad: MARGIN,
                };
                painter.resize(1.5, layout.window()).expect("surfaces");
                for (label, view, stretch) in &views {
                    let (level, mute, alert) = match view {
                        Osd::Volume {
                            volume_pct, muted, ..
                        } => (volume_pct / 100.0, f32::from(u8::from(*muted)), 0.0),
                        Osd::Dictation(_) => (0.0, 0.0, 0.0),
                        Osd::Status { alert, .. } => (0.0, 0.0, f32::from(u8::from(*alert))),
                    };
                    let status_width = match view {
                        Osd::Status { title, value, .. } => {
                            painter.status_width(title, value, look)
                        }
                        Osd::Volume { .. } | Osd::Dictation(_) => STATUS_MIN_W,
                    };
                    let scene = Scene {
                        view,
                        enter: 1.0,
                        level,
                        mute,
                        alert,
                        stretch: *stretch,
                        status_width,
                        vu: 0.5,
                        vu_peak: 0.0,
                        shimmer: 0.5,
                    };
                    painter
                        .paint(&scene, layout, Some(0x3a4a5c))
                        .expect("paint");
                    painter
                        .save_png(&directory.join(format!(
                            "2026-10-18-osd-{look_name}-{style_name}-{label}.png"
                        )))
                        .expect("png");
                }
                // The entrance: frames along the spring, the bounce included.
                let view = volume("Main", "blue", "speaker", 67.0, false);
                let mut enter = Spring {
                    target: 1.0,
                    ..Spring::default()
                };
                for frame in 0..8 {
                    let scene = Scene {
                        view: &view,
                        enter: enter.value,
                        level: 0.67,
                        mute: 0.0,
                        alert: 0.0,
                        stretch: 0.0,
                        status_width: STATUS_MIN_W,
                        vu: 0.5,
                        vu_peak: 0.0,
                        shimmer: 0.5,
                    };
                    painter
                        .paint(&scene, layout, Some(0x3a4a5c))
                        .expect("paint");
                    painter
                        .save_png(&directory.join(format!(
                            "2026-10-18-osd-{look_name}-{style_name}-enter-{frame}.png"
                        )))
                        .expect("png");
                    enter.step(1.0 / 30.0, ENTER.0, ENTER.1);
                }
            }
        }
    }
}
