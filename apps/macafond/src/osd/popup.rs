//! Windows popup: a layered, click-through, topmost window owned by its own thread. Direct2D draws
//! each frame into a WIC bitmap, `UpdateLayeredWindow` shows it; springs drive every motion and the
//! loop paces itself on `DwmFlush` while anything moves.
use super::dictation::{State as DictationState, View as DictationView, Vu};
use super::icons::ICONS;
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

const WAKE: u32 = WM_APP + 1;
/// How long the popup stays after the last change.
const HOLD: Duration = Duration::from_millis(1600);

// Layout in DIPs. The panel keeps `MARGIN` from its edge of the work area plus the offset from
// Settings; the window around it leaves room for the shadow, the overscroll and the slide.
const MARGIN: f32 = 20.0;
const SHADOW: f32 = 18.0;
const SLIDE: f32 = 28.0;
/// How far an overscrolled slider stretches.
const STRETCH: f32 = 12.0;
// Vertical style (Android-like): a pill-shaped slider.
const VOLUME_W: f32 = 68.0;
const VOLUME_H: f32 = 268.0;
// Horizontal style (Windows 11-like): a bar with the flyouts' small corners.
const BAR_W: f32 = 340.0;
const BAR_H: f32 = 60.0;
const BAR_RADIUS: f32 = 8.0;
// The other actions, in both styles.
const STATUS_H: f32 = 64.0;
const STATUS_MIN_W: f32 = 196.0;
const STATUS_MAX_W: f32 = 340.0;

// Design tokens (docs/design/design-system.md).
const BG_RAISED: u32 = 0x101014;
const INK: u32 = 0xf2f1ee;
const WHITE: u32 = 0xffffff;
const ACCENT: u32 = 0x2d81ff;
const DANGER: u32 = 0xff6259;

// The Analog dictation popup: a small, quiet VU meter (§9.9.3).
const ANALOG_W: f32 = 128.0;
const ANALOG_H: f32 = 60.0;
/// Bars of the Flow pill: 15 bars of 3 px every 6 px.
const FLOW_BARS_W: f32 = 87.0;
/// Needle travel 0…1 for a linear VU amplitude (1 = 0 VU): linear in voltage from −20 to
/// +3 VU, so 0 VU sits about two thirds along, as on real meters.
fn vu_travel(amplitude: f32) -> f32 {
    let (low, high) = (0.1f32, 10f32.powf(3.0 / 20.0));
    ((amplitude - low) / (high - low)).clamp(-0.02, 1.08)
}

fn dictation_size(view: &DictationView) -> (f32, f32) {
    if view.look == macafon_ipc::DictationPopupLook::Analog {
        return (ANALOG_W, ANALOG_H);
    }
    // Only the voice: no timer, no language; hands-free adds its two buttons on the right.
    match view.state {
        DictationState::Listening if view.hands_free => (150.0, 32.0),
        DictationState::Listening => (116.0, 32.0),
        DictationState::Transcribing => (88.0, 28.0),
        DictationState::Done => (36.0, 28.0),
        _ => (180.0, 32.0),
    }
}

/// `--app-<color>` fill and `--app-<color>-ink` text for each of `CHANNEL_COLORS`.
fn channel_colors(color: &str) -> (u32, u32) {
    match color {
        "violet" => (0x8b5cf6, 0xb69cff),
        "pink" => (0xe0368f, 0xff7ab8),
        "orange" => (0xe8573f, 0xff8a70),
        "amber" => (0xf2a516, 0xffc14d),
        "teal" => (0x12a58a, 0x3fd6b8),
        "lime" => (0x7bb51c, 0xa6e04a),
        _ => (0x2f7bf5, 0x7fb0ff),
    }
}

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
                    if let Some(Osd::Dictation(view)) = &popup.view {
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
            moving = popup.visible && popup.advance(now, dt);
            if popup.visible && (moving || dirty) {
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

/// A damped spring; `step` integrates in small substeps so a long frame cannot blow it up.
#[derive(Debug, Clone, Copy, Default)]
struct Spring {
    value: f32,
    velocity: f32,
    target: f32,
}

impl Spring {
    fn snap(&mut self, target: f32) {
        *self = Self {
            value: target,
            velocity: 0.0,
            target,
        };
    }

    fn step(&mut self, dt: f32, frequency_hz: f32, damping: f32) {
        let omega = std::f32::consts::TAU * frequency_hz;
        let steps = (dt * 480.0).ceil().max(1.0);
        let h = dt / steps;
        for _ in 0..steps as usize {
            let acceleration =
                -omega * omega * (self.value - self.target) - 2.0 * damping * omega * self.velocity;
            self.velocity += acceleration * h;
            self.value += self.velocity * h;
        }
    }

    fn settled(&self) -> bool {
        (self.value - self.target).abs() < 1e-3 && self.velocity.abs() < 1e-2
    }
}

/// Spring tunings: (frequency in Hz, damping ratio).
const ENTER: (f32, f32) = (3.2, 0.62);
const EXIT: (f32, f32) = (3.6, 1.0);
const LEVEL: (f32, f32) = (7.0, 1.0);
const FADE: (f32, f32) = (5.0, 1.0);
const BOUNCE: (f32, f32) = (4.0, 0.35);
const KICK: f32 = 18.0;

/// Where the panel sits in the window and which way it slides in.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Layout {
    setup: OsdConfig,
    /// From the window's edge to the panel: `MARGIN`, plus room for the slide once the offset
    /// moves the panel away from the screen edge (the window never leaves the work area).
    pad: f32,
}

impl Layout {
    fn window(&self) -> (f32, f32) {
        match self.setup.style {
            OsdStyle::Vertical => (
                STATUS_MAX_W + self.pad + SHADOW,
                VOLUME_H + 2.0 * (SHADOW + STRETCH),
            ),
            OsdStyle::Horizontal => (
                STATUS_MAX_W + 2.0 * (SHADOW + STRETCH),
                176.0 + self.pad + SHADOW,
            ),
        }
    }

    /// The panel in a window of `size`: its bounds, the middle of its edge side (it grows from
    /// there) and the direction it slides in from.
    fn place(&self, size: (f32, f32), panel: (f32, f32)) -> (D2D_RECT_F, (f32, f32), (f32, f32)) {
        let ((w, h), (pw, ph), pad) = (size, panel, self.pad);
        match self.setup.edge {
            OsdEdge::Right => (
                rect(w - pad - pw, (h - ph) / 2.0, w - pad, (h + ph) / 2.0),
                (w - pad, h / 2.0),
                (1.0, 0.0),
            ),
            OsdEdge::Left => (
                rect(pad, (h - ph) / 2.0, pad + pw, (h + ph) / 2.0),
                (pad, h / 2.0),
                (-1.0, 0.0),
            ),
            OsdEdge::Bottom => (
                rect((w - pw) / 2.0, h - pad - ph, (w + pw) / 2.0, h - pad),
                (w / 2.0, h - pad),
                (0.0, 1.0),
            ),
            OsdEdge::Top => (
                rect((w - pw) / 2.0, pad, (w + pw) / 2.0, pad + ph),
                (w / 2.0, pad),
                (0.0, -1.0),
            ),
        }
    }
}

struct Popup {
    window: HWND,
    painter: Painter,
    layout: Layout,
    view: Option<Osd>,
    visible: bool,
    shown: bool,
    exiting: bool,
    changed_at: Instant,
    /// Window origin in screen pixels.
    origin: POINT,
    enter: Spring,
    level: Spring,
    mute: Spring,
    alert: Spring,
    stretch: Spring,
    status_width: f32,
    vu: Vu,
    /// The Analog needle's peak, held and let down slowly: a faint ghost needle.
    vu_peak: f32,
    shimmer: f32,
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
            view: None,
            visible: false,
            shown: false,
            exiting: false,
            changed_at: Instant::now(),
            origin: POINT::default(),
            enter: Spring::default(),
            level: Spring::default(),
            mute: Spring::default(),
            alert: Spring::default(),
            stretch: Spring::default(),
            status_width: STATUS_MIN_W,
            vu: Vu::default(),
            vu_peak: 0.0,
            shimmer: 0.0,
        }
    }

    fn receive(&mut self, osd: Osd, setup: OsdConfig, now: Instant) {
        if matches!(&osd, Osd::Dictation(v) if v.state == DictationState::Cancelled) {
            self.hide();
            self.view = None;
            return;
        }
        let same = match (&self.view, &osd) {
            (Some(Osd::Dictation(_)), Osd::Dictation(_)) => true,
            (Some(Osd::Volume { name: a, .. }), Osd::Volume { name: b, .. }) => a == b,
            (Some(Osd::Status { title: a, .. }), Osd::Status { title: b, .. }) => a == b,
            _ => false,
        };
        // Settings changing the setup moves a visible popup right away.
        if !self.visible || setup != self.layout.setup {
            if let Err(error) = self.place(setup, matches!(osd, Osd::Dictation(_))) {
                tracing::debug!(%error, "Cannot place hotkey popup");
                return;
            }
        }
        if !self.visible {
            self.enter.snap(0.0);
            self.visible = true;
        } else if !same {
            // Another channel or action: replay the tail of the entrance.
            self.enter.value = self.enter.value.min(0.8);
        }
        self.enter.target = 1.0;
        self.exiting = false;
        self.changed_at = now;
        match &osd {
            Osd::Dictation(_) => {}
            Osd::Volume {
                volume_pct, muted, ..
            } => {
                let target = (volume_pct / 100.0).clamp(0.0, 1.0);
                // Pressing on at a limit stretches the slider like Android's overscroll.
                // A held key keeps pushing: the push is held steady, not added up per update.
                if same && target == self.level.target && (target == 0.0 || target == 1.0) {
                    self.stretch.velocity = if target == 1.0 {
                        self.stretch.velocity.max(KICK)
                    } else {
                        self.stretch.velocity.min(-KICK)
                    };
                }
                if same {
                    self.level.target = target;
                    self.mute.target = f32::from(u8::from(*muted));
                } else {
                    self.level.snap(target);
                    self.mute.snap(f32::from(u8::from(*muted)));
                }
            }
            Osd::Status {
                title,
                value,
                alert,
                ..
            } => {
                let target = f32::from(u8::from(*alert));
                if same {
                    self.alert.target = target;
                } else {
                    self.alert.snap(target);
                }
                self.status_width = self.painter.status_width(title, value, setup.look);
            }
        }
        // SAFETY: this thread owns the window; only its click-through style changes.
        let clickable = matches!(&osd, Osd::Dictation(view) if view.hands_free && view.state == DictationState::Listening);
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
        self.view = Some(osd);
        if !animations_enabled() {
            for spring in [
                &mut self.enter,
                &mut self.level,
                &mut self.mute,
                &mut self.alert,
            ] {
                spring.snap(spring.target);
            }
            self.stretch.snap(0.0);
        }
    }

    /// Moves every spring by `dt`; false once all rest.
    fn advance(&mut self, now: Instant, dt: f32) -> bool {
        if !self.exiting && now - self.changed_at >= self.hold() {
            self.exiting = true;
            self.enter.target = 0.0;
            if !animations_enabled() {
                self.enter.snap(0.0);
            }
        }
        let (frequency, damping) = if self.exiting { EXIT } else { ENTER };
        self.enter.step(dt, frequency, damping);
        if let Some(Osd::Dictation(view)) = &self.view {
            let rms = if view.state == DictationState::Listening {
                view.rms_dbfs
            } else {
                -120.0
            };
            if animations_enabled() {
                self.vu.step(rms, dt);
                self.shimmer += dt;
                let travel = vu_travel(self.vu.value);
                self.vu_peak = travel.max(self.vu_peak - dt * 0.35);
            } else {
                self.vu.value = 10.0f32.powf((rms + 18.0) / 20.0).clamp(0.0, 1.6);
            }
        }
        self.level.step(dt, LEVEL.0, LEVEL.1);
        self.mute.step(dt, FADE.0, FADE.1);
        self.alert.step(dt, FADE.0, FADE.1);
        self.stretch.step(dt, BOUNCE.0, BOUNCE.1);
        if self.exiting && self.enter.value <= 0.01 {
            self.hide();
            return false;
        }
        if matches!(&self.view, Some(Osd::Dictation(v)) if matches!(v.state, DictationState::Listening | DictationState::Transcribing))
        {
            return true;
        }
        ![self.enter, self.level, self.mute, self.alert, self.stretch]
            .iter()
            .all(Spring::settled)
    }

    /// Wait until the hold ends while shown, forever while hidden.
    fn timeout(&self, now: Instant) -> u32 {
        if !self.visible || self.exiting {
            return u32::MAX; // INFINITE
        }
        let left = self.hold().saturating_sub(now - self.changed_at);
        u32::try_from(left.as_millis()).unwrap_or(u32::MAX).max(1)
    }

    fn hold(&self) -> Duration {
        if let Some(Osd::Dictation(view)) = &self.view {
            if matches!(view.state, DictationState::Done | DictationState::Copied) {
                return Duration::from_millis(
                    if view.look == macafon_ipc::DictationPopupLook::Analog {
                        700
                    } else {
                        400
                    },
                );
            }
        }
        HOLD
    }

    fn hide(&mut self) {
        DICTATION_BUTTONS.with(|b| b.set(None));
        self.visible = false;
        self.shown = false;
        self.exiting = false;
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
            let margin = if dictation { 48.0 } else { MARGIN };
            let layout = Layout {
                setup,
                pad: margin + (offset as f32 / scale).min(SLIDE + SHADOW),
            };
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
        let Some(view) = &self.view else {
            return Ok(());
        };
        let scene = Scene {
            view,
            enter: self.enter.value,
            level: self.level.value.clamp(0.0, 1.0),
            mute: self.mute.value.clamp(0.0, 1.0),
            alert: self.alert.value.clamp(0.0, 1.0),
            stretch: self.stretch.value,
            status_width: self.status_width,
            vu: self.vu.value,
            vu_peak: self.vu_peak,
            shimmer: self.shimmer,
        };
        self.painter.paint(&scene, self.layout, None)?;
        let buttons = if let Osd::Dictation(view) = view {
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
        let opacity = (self.enter.value * 1.4).clamp(0.0, 1.0);
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

/// One frame: `enter` 0 → 1 (springs may overshoot), the rest in 0..=1 except `stretch`.
struct Scene<'a> {
    vu: f32,
    vu_peak: f32,
    shimmer: f32,
    view: &'a Osd,
    enter: f32,
    level: f32,
    mute: f32,
    alert: f32,
    stretch: f32,
    status_width: f32,
}

fn rgba(hex: u32, a: f32) -> D2D1_COLOR_F {
    let channel = |shift: u32| ((hex >> shift) & 0xff) as f32 / 255.0;
    D2D1_COLOR_F {
        r: channel(16),
        g: channel(8),
        b: channel(0),
        a,
    }
}

fn mix(from: D2D1_COLOR_F, to: D2D1_COLOR_F, t: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: from.r + (to.r - from.r) * t,
        g: from.g + (to.g - from.g) * t,
        b: from.b + (to.b - from.b) * t,
        a: from.a + (to.a - from.a) * t,
    }
}

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left,
        top,
        right,
        bottom,
    }
}

fn rounded(rect: D2D_RECT_F, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect,
        radiusX: radius,
        radiusY: radius,
    }
}

const IDENTITY: Matrix3x2 = Matrix3x2 {
    M11: 1.0,
    M12: 0.0,
    M21: 0.0,
    M22: 1.0,
    M31: 0.0,
    M32: 0.0,
};

/// Direct2D over a WIC bitmap plus the GDI DIB `UpdateLayeredWindow` reads from.
struct Painter {
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    wic: IWICImagingFactory,
    bitmap: Option<IWICBitmap>,
    target: Option<ID2D1RenderTarget>,
    /// SVG icons need `ID2D1DeviceContext5` (Windows 10 1703+); without it icons are skipped.
    context: Option<ID2D1DeviceContext5>,
    dib: HBITMAP,
    dc: HDC,
    bits: *mut u8,
    scale: f32,
    /// The window in DIPs; `width` × `height` in pixels.
    size: (f32, f32),
    width: u32,
    height: u32,
    /// The panel's transform for this frame: icons are placed relative to it.
    panel: Matrix3x2,
    icons: HashMap<(&'static str, u32, u32), ID2D1SvgDocument>,
    mono: IDWriteTextFormat,
    mono_center: IDWriteTextFormat,
    hud: IDWriteTextFormat,
    hud_center: IDWriteTextFormat,
    percent: IDWriteTextFormat,
    name: IDWriteTextFormat,
    label: IDWriteTextFormat,
    title: IDWriteTextFormat,
    value: IDWriteTextFormat,
    dictation_hud: IDWriteTextFormat,
    dictation_hud_center: IDWriteTextFormat,
    dictation_serif: IDWriteTextFormat,
    dictation_body: IDWriteTextFormat,
    _fonts: super::fonts::Fonts,
}

impl Painter {
    fn new() -> Result<Self> {
        // SAFETY: factories for this thread only.
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let wic: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let display = w!("Segoe UI Variable Display");
            let text = w!("Segoe UI Variable Text");
            let mut fonts = None;
            dwrite.GetSystemFontCollection(&mut fonts, false)?;
            let mut family = w!("Consolas");
            if let Some(fonts) = fonts {
                for candidate in [w!("Geist Mono"), w!("Cascadia Mono"), w!("Consolas")] {
                    let mut index = 0;
                    let mut exists = BOOL(0);
                    fonts.FindFamilyName(candidate, &mut index, &mut exists)?;
                    if exists.as_bool() {
                        family = candidate;
                        break;
                    }
                }
            }
            let mono = format(&dwrite, family, DWRITE_FONT_WEIGHT_MEDIUM, 13.0, false)?;
            let mono_center = format(&dwrite, family, DWRITE_FONT_WEIGHT_MEDIUM, 13.0, true)?;
            let hud = format(&dwrite, family, DWRITE_FONT_WEIGHT_MEDIUM, 11.0, false)?;
            let hud_center = format(&dwrite, family, DWRITE_FONT_WEIGHT_MEDIUM, 11.0, true)?;
            let percent = format(&dwrite, display, DWRITE_FONT_WEIGHT_SEMI_BOLD, 15.0, true)?;
            let name = format(&dwrite, text, DWRITE_FONT_WEIGHT_SEMI_BOLD, 12.0, true)?;
            let label = format(&dwrite, text, DWRITE_FONT_WEIGHT_SEMI_BOLD, 12.0, false)?;
            let title = format(&dwrite, text, DWRITE_FONT_WEIGHT_NORMAL, 12.0, false)?;
            let value = format(&dwrite, display, DWRITE_FONT_WEIGHT_SEMI_BOLD, 16.0, false)?;
            let fonts = super::fonts::Fonts::new(&dwrite)?;
            let dictation_hud = fonts.format(
                w!("Geist Mono"),
                DWRITE_FONT_WEIGHT_MEDIUM,
                10.0,
                false,
                false,
            )?;
            let dictation_hud_center = fonts.format(
                w!("Geist Mono"),
                DWRITE_FONT_WEIGHT_MEDIUM,
                10.0,
                true,
                false,
            )?;
            let dictation_serif = fonts.format(
                w!("Instrument Serif"),
                DWRITE_FONT_WEIGHT_NORMAL,
                17.0,
                true,
                true,
            )?;
            let dictation_body =
                fonts.format(w!("Inter"), DWRITE_FONT_WEIGHT_NORMAL, 12.0, false, false)?;
            Ok(Self {
                d2d,
                dwrite,
                wic,
                bitmap: None,
                target: None,
                context: None,
                dib: HBITMAP::default(),
                dc: CreateCompatibleDC(None),
                bits: std::ptr::null_mut(),
                scale: 0.0,
                size: (0.0, 0.0),
                width: 0,
                height: 0,
                panel: IDENTITY,
                icons: HashMap::new(),
                mono,
                mono_center,
                hud,
                hud_center,
                percent,
                name,
                label,
                title,
                value,
                dictation_hud,
                dictation_hud_center,
                dictation_serif,
                dictation_body,
                _fonts: fonts,
            })
        }
    }

    /// Surfaces for a window of `size` DIPs at `scale`; kept while neither changes.
    fn resize(&mut self, scale: f32, size: (f32, f32)) -> Result<()> {
        if scale == self.scale && size == self.size && self.target.is_some() {
            return Ok(());
        }
        let width = (size.0 * scale).ceil() as u32;
        let height = (size.1 * scale).ceil() as u32;
        // SAFETY: the old DIB is no longer selected once replaced; WIC and D2D copy nothing out.
        unsafe {
            let bitmap = self.wic.CreateBitmap(
                width,
                height,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapCacheOnLoad,
            )?;
            let target = self.d2d.CreateWicBitmapRenderTarget(
                &bitmap,
                &D2D1_RENDER_TARGET_PROPERTIES {
                    r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.0 * scale,
                    dpiY: 96.0 * scale,
                    ..Default::default()
                },
            )?;
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width as i32,
                    biHeight: -(height as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let dib = CreateDIBSection(Some(self.dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
            SelectObject(self.dc, dib.into());
            if !self.dib.is_invalid() {
                let _ = DeleteObject(self.dib.into());
            }
            self.dib = dib;
            self.bits = bits.cast();
            self.context = target.cast().ok();
            if self.context.is_none() {
                tracing::debug!("Direct2D SVG unavailable; popup icons are hidden");
            }
            self.icons.clear();
            self.bitmap = Some(bitmap);
            self.target = Some(target);
        }
        self.scale = scale;
        self.size = size;
        self.width = width;
        self.height = height;
        Ok(())
    }

    fn text_width(&self, text: &str, format: &IDWriteTextFormat) -> f32 {
        let text: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: the layout reads `text` during the call only.
        unsafe {
            let mut metrics = DWRITE_TEXT_METRICS::default();
            self.dwrite
                .CreateTextLayout(&text, format, STATUS_MAX_W, 100.0)
                .and_then(|layout| layout.GetMetrics(&mut metrics))
                .map_or(STATUS_MAX_W, |()| metrics.widthIncludingTrailingWhitespace)
        }
    }

    fn status_width(&self, title: &str, value: &str, look: OsdLook) -> f32 {
        let text = if look == OsdLook::Sketch {
            let title = title.to_uppercase();
            (self.text_width(&title, &self.hud) + title.chars().count() as f32 * 1.65)
                .max(self.text_width(value, &self.mono))
        } else {
            self.text_width(title, &self.title)
                .max(self.text_width(value, &self.value))
        };
        (64.0 + text + 24.0).clamp(STATUS_MIN_W, STATUS_MAX_W)
    }

    fn paint(&mut self, scene: &Scene, layout: Layout, backdrop: Option<u32>) -> Result<()> {
        let Some(target) = self.target.clone() else {
            return Ok(());
        };
        let horizontal = layout.setup.style == OsdStyle::Horizontal;
        let panel = match scene.view {
            Osd::Volume { .. } if horizontal => (BAR_W, BAR_H),
            Osd::Volume { .. } => (VOLUME_W, VOLUME_H),
            Osd::Status { .. } => (scene.status_width, STATUS_H),
            Osd::Dictation(view) => dictation_size(view),
        };
        let (bounds, (pivot_x, pivot_y), (toward_x, toward_y)) = layout.place(self.size, panel);
        // Grow from the panel's edge side and slide in from the screen edge; an overshoot of
        // `enter` past 1 is the bounce.
        let e = scene.enter;
        let scale = 0.86 + 0.14 * e;
        self.panel = Matrix3x2 {
            M11: scale,
            M22: scale,
            M31: (1.0 - scale) * pivot_x + (1.0 - e) * SLIDE * toward_x,
            M32: (1.0 - scale) * pivot_y + (1.0 - e) * SLIDE * toward_y,
            ..IDENTITY
        };
        // SAFETY: drawing on this thread's render target between BeginDraw and EndDraw.
        unsafe {
            target.BeginDraw();
            target.SetTransform(&IDENTITY);
            target.Clear(Some(&backdrop.map_or(rgba(0, 0.0), |hex| rgba(hex, 1.0))));
            target.SetTransform(&self.panel);
            let drawn = if let Osd::Dictation(view) = scene.view {
                self.dictation(&target, scene, bounds, view)
            } else if layout.setup.look == OsdLook::Sketch {
                self.sketch(&target, scene, bounds, horizontal)
            } else {
                match scene.view {
                    Osd::Dictation(view) => self.dictation(&target, scene, bounds, view),
                    Osd::Volume {
                        name, color, icon, ..
                    } if horizontal => self.bar(&target, scene, bounds, name, color, icon),
                    Osd::Volume {
                        name, color, icon, ..
                    } => self.volume(&target, scene, bounds, name, color, icon),
                    Osd::Status { .. } => {
                        let radius = if horizontal {
                            BAR_RADIUS
                        } else {
                            STATUS_H / 2.0
                        };
                        self.status(&target, scene, bounds, radius)
                    }
                }
            };
            let ended = target.EndDraw(None, None);
            drawn.and(ended)
        }
    }

    /// Square frame and ruler, using the same animated scene as Classic.
    unsafe fn sketch(
        &mut self,
        target: &ID2D1RenderTarget,
        scene: &Scene,
        bounds: D2D_RECT_F,
        horizontal: bool,
    ) -> Result<()> {
        let positive = scene.stretch.max(0.0) * STRETCH / KICK * 4.0;
        let negative = (-scene.stretch).max(0.0) * STRETCH / KICK * 4.0;
        let bounds = if matches!(scene.view, Osd::Volume { .. }) {
            if horizontal {
                rect(
                    bounds.left - negative,
                    bounds.top,
                    bounds.right + positive,
                    bounds.bottom,
                )
            } else {
                rect(
                    bounds.left,
                    bounds.top - positive,
                    bounds.right,
                    bounds.bottom + negative,
                )
            }
        } else {
            bounds
        };
        // SAFETY: called between BeginDraw and EndDraw on this thread's target.
        unsafe {
            let surface = target.CreateSolidColorBrush(&rgba(BG_RAISED, 1.0), None)?;
            let line = target.CreateSolidColorBrush(&rgba(INK, 0.15), None)?;
            let strong = target.CreateSolidColorBrush(&rgba(INK, 0.4), None)?;
            target.FillRectangle(&bounds, &surface);
            target.DrawRectangle(&bounds, &line, 1.0, None);
            for x in [bounds.left, bounds.right] {
                for y in [bounds.top, bounds.bottom] {
                    target.DrawLine(
                        Vector2 { X: x - 5.5, Y: y },
                        Vector2 { X: x + 5.5, Y: y },
                        &strong,
                        1.0,
                        None,
                    );
                    target.DrawLine(
                        Vector2 { X: x, Y: y - 5.5 },
                        Vector2 { X: x, Y: y + 5.5 },
                        &strong,
                        1.0,
                        None,
                    );
                }
            }
            match scene.view {
                Osd::Dictation(view) => self.dictation(target, scene, bounds, view)?,
                Osd::Volume {
                    name, color, icon, ..
                } => {
                    let (fill, ink) = channel_colors(color);
                    let track = if horizontal {
                        rect(
                            bounds.left + 54.0,
                            bounds.top + 34.0,
                            bounds.right - 64.0,
                            bounds.bottom - 12.0,
                        )
                    } else {
                        rect(
                            bounds.left + 14.0,
                            bounds.top + 46.0,
                            bounds.right - 14.0,
                            bounds.bottom - 46.0,
                        )
                    };
                    let sunken = target.CreateSolidColorBrush(&rgba(0x060607, 1.0), None)?;
                    let fill =
                        target.CreateSolidColorBrush(&rgba(fill, 1.0 - 0.75 * scene.mute), None)?;
                    target.FillRectangle(&track, &sunken);
                    let level = scene.level.clamp(0.0, 1.0);
                    let head = if horizontal {
                        track.left + (track.right - track.left) * level
                    } else {
                        track.bottom - (track.bottom - track.top) * level
                    };
                    let filled = if horizontal {
                        rect(track.left, track.top, head, track.bottom)
                    } else {
                        rect(track.left, head, track.right, track.bottom)
                    };
                    target.FillRectangle(&filled, &fill);
                    target.DrawRectangle(&track, &line, 1.0, None);
                    for tick in 0..=10 {
                        let t = tick as f32 / 10.0;
                        let length = if tick % 5 == 0 { 7.0 } else { 4.0 };
                        let (from, to) = if horizontal {
                            let x = track.left + (track.right - track.left) * t;
                            (
                                Vector2 {
                                    X: x,
                                    Y: track.bottom,
                                },
                                Vector2 {
                                    X: x,
                                    Y: track.bottom - length,
                                },
                            )
                        } else {
                            let y = track.bottom - (track.bottom - track.top) * t;
                            (
                                Vector2 {
                                    X: track.left,
                                    Y: y,
                                },
                                Vector2 {
                                    X: track.left + length,
                                    Y: y,
                                },
                            )
                        };
                        target.DrawLine(from, to, &strong, 1.0, None);
                    }
                    let head_brush =
                        target.CreateSolidColorBrush(&rgba(ink, 1.0 - 0.5 * scene.mute), None)?;
                    let (from, to) = if horizontal {
                        (
                            Vector2 {
                                X: head,
                                Y: track.top - 3.0,
                            },
                            Vector2 {
                                X: head,
                                Y: track.bottom + 3.0,
                            },
                        )
                    } else {
                        (
                            Vector2 {
                                X: track.left - 3.0,
                                Y: head,
                            },
                            Vector2 {
                                X: track.right + 3.0,
                                Y: head,
                            },
                        )
                    };
                    target.DrawLine(from, to, &head_brush, 2.0, None);
                    let (caption, value, format) = if horizontal {
                        self.icon(
                            icon,
                            ink,
                            1.0 - 0.4 * scene.mute,
                            bounds.left + 27.0,
                            (bounds.top + bounds.bottom) / 2.0,
                            20.0,
                        )?;
                        (
                            rect(track.left, bounds.top + 8.0, track.right, bounds.top + 29.0),
                            rect(
                                bounds.right - 60.0,
                                bounds.top,
                                bounds.right - 8.0,
                                bounds.bottom,
                            ),
                            &self.mono_center,
                        )
                    } else {
                        (
                            rect(
                                bounds.left + 4.0,
                                bounds.bottom - 38.0,
                                bounds.right - 4.0,
                                bounds.bottom - 12.0,
                            ),
                            rect(
                                bounds.left + 4.0,
                                bounds.top + 12.0,
                                bounds.right - 4.0,
                                bounds.top + 36.0,
                            ),
                            &self.mono_center,
                        )
                    };
                    self.hud_text(target, name, caption, rgba(ink, 1.0), !horizontal)?;
                    self.text(
                        target,
                        &format!("{:.0}%", level * 100.0),
                        format,
                        value,
                        mix(rgba(INK, 1.0), rgba(DANGER, 1.0), scene.mute),
                    )?;
                }
                Osd::Status {
                    icon, title, value, ..
                } => {
                    let x = bounds.left + STATUS_H / 2.0;
                    let y = (bounds.top + bounds.bottom) / 2.0;
                    let badge = target.CreateSolidColorBrush(
                        &mix(rgba(ACCENT, 1.0), rgba(DANGER, 1.0), scene.alert),
                        None,
                    )?;
                    target.FillRectangle(&rect(x - 20.0, y - 20.0, x + 20.0, y + 20.0), &badge);
                    let id = match icon {
                        StatusIcon::Mic => "mic",
                        StatusIcon::MicOff => "mic-off",
                        StatusIcon::Processing => "processing",
                        StatusIcon::Preset => "preset",
                    };
                    self.icon(id, WHITE, 1.0, x, y, 22.0)?;
                    self.hud_text(
                        target,
                        title,
                        rect(
                            bounds.left + STATUS_H,
                            bounds.top + 9.0,
                            bounds.right - 16.0,
                            bounds.top + 30.0,
                        ),
                        rgba(INK, 0.6),
                        false,
                    )?;
                    self.text(
                        target,
                        value,
                        &self.mono,
                        rect(
                            bounds.left + STATUS_H,
                            bounds.top + 29.0,
                            bounds.right - 16.0,
                            bounds.bottom - 8.0,
                        ),
                        rgba(INK, 1.0),
                    )?;
                }
            }
        }
        Ok(())
    }

    unsafe fn hud_text(
        &self,
        target: &ID2D1RenderTarget,
        text: &str,
        area: D2D_RECT_F,
        color: D2D1_COLOR_F,
        centered: bool,
    ) -> Result<()> {
        let text: Vec<u16> = text.to_uppercase().encode_utf16().collect();
        // SAFETY: the layout copies the text and is drawn on this thread's active target.
        unsafe {
            let layout = self.dwrite.CreateTextLayout(
                &text,
                if centered {
                    &self.hud_center
                } else {
                    &self.hud
                },
                area.right - area.left,
                area.bottom - area.top,
            )?;
            let spaced: IDWriteTextLayout1 = layout.cast()?;
            spaced.SetCharacterSpacing(
                0.0,
                1.65,
                0.0,
                DWRITE_TEXT_RANGE {
                    startPosition: 0,
                    length: text.len() as u32,
                },
            )?;
            let brush = target.CreateSolidColorBrush(&color, None)?;
            target.DrawTextLayout(
                Vector2 {
                    X: area.left,
                    Y: area.top,
                },
                &layout,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
        Ok(())
    }

    /// A raised pill with a soft shadow and a hairline border.
    unsafe fn dictation_hud_text(
        &self,
        target: &ID2D1RenderTarget,
        text: &str,
        area: D2D_RECT_F,
        color: D2D1_COLOR_F,
        centered: bool,
    ) -> Result<()> {
        let text: Vec<u16> = text.to_uppercase().encode_utf16().collect();
        // SAFETY: the layout copies the text and is drawn on this thread's active target.
        unsafe {
            let layout = self.dwrite.CreateTextLayout(
                &text,
                if centered {
                    &self.dictation_hud_center
                } else {
                    &self.dictation_hud
                },
                area.right - area.left,
                area.bottom - area.top,
            )?;
            let spaced: IDWriteTextLayout1 = layout.cast()?;
            spaced.SetCharacterSpacing(
                0.0,
                1.65,
                0.0,
                DWRITE_TEXT_RANGE {
                    startPosition: 0,
                    length: text.len() as u32,
                },
            )?;
            let brush = target.CreateSolidColorBrush(&color, None)?;
            target.DrawTextLayout(
                Vector2 {
                    X: area.left,
                    Y: area.top,
                },
                &layout,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
        Ok(())
    }

    /// A raised pill with a soft shadow and a hairline border.
    unsafe fn card(
        &self,
        target: &ID2D1RenderTarget,
        bounds: D2D_RECT_F,
        radius: f32,
    ) -> Result<()> {
        // SAFETY: called between BeginDraw and EndDraw.
        unsafe {
            let shadow = target.CreateSolidColorBrush(&rgba(0, 1.0), None)?;
            for ring in 0..6 {
                let grow = ring as f32 * 3.0;
                shadow.SetOpacity(0.09 * (1.0 - ring as f32 / 6.0));
                let spread = rect(
                    bounds.left - grow,
                    bounds.top - grow + 6.0,
                    bounds.right + grow,
                    bounds.bottom + grow + 6.0,
                );
                target.FillRoundedRectangle(&rounded(spread, radius + grow), &shadow);
            }
            let fill = target.CreateSolidColorBrush(&rgba(BG_RAISED, 0.96), None)?;
            target.FillRoundedRectangle(&rounded(bounds, radius), &fill);
            let line = target.CreateSolidColorBrush(&rgba(INK, 0.15), None)?;
            target.DrawRoundedRectangle(&rounded(bounds, radius), &line, 1.0, None);
        }
        Ok(())
    }

    unsafe fn text(
        &self,
        target: &ID2D1RenderTarget,
        text: &str,
        format: &IDWriteTextFormat,
        area: D2D_RECT_F,
        color: D2D1_COLOR_F,
    ) -> Result<()> {
        let text: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: called between BeginDraw and EndDraw; `text` outlives the call.
        unsafe {
            let brush = target.CreateSolidColorBrush(&color, None)?;
            target.DrawText(
                &text,
                format,
                &area,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
        Ok(())
    }

    unsafe fn volume(
        &mut self,
        target: &ID2D1RenderTarget,
        scene: &Scene,
        bounds: D2D_RECT_F,
        name: &str,
        color: &str,
        icon: &str,
    ) -> Result<()> {
        let up = scene.stretch.max(0.0) * STRETCH / KICK * 4.0;
        let down = (-scene.stretch).max(0.0) * STRETCH / KICK * 4.0;
        let bounds = rect(
            bounds.left,
            bounds.top - up,
            bounds.right,
            bounds.bottom + down,
        );
        let (fill, ink) = channel_colors(color);
        let grey = rgba(INK, 0.32);
        // SAFETY: called between BeginDraw and EndDraw.
        unsafe {
            self.card(target, bounds, VOLUME_W / 2.0)?;
            // What it is set to: the percentage, crossfading to "Muted".
            let label = rect(
                bounds.left,
                bounds.top + 12.0,
                bounds.right,
                bounds.top + 38.0,
            );
            let percent = format!("{:.0}%", scene.level * 100.0);
            self.text(
                target,
                &percent,
                &self.percent,
                label,
                rgba(INK, 1.0 - scene.mute),
            )?;
            self.text(
                target,
                "Muted",
                &self.percent,
                label,
                rgba(DANGER, scene.mute),
            )?;
            // The slider: an empty track and a fill that keeps at least the icon's circle.
            let track = rect(
                bounds.left + 10.0,
                bounds.top + 46.0,
                bounds.right - 10.0,
                bounds.bottom - 44.0,
            );
            let thickness = track.right - track.left;
            let empty = target.CreateSolidColorBrush(&rgba(INK, 0.10), None)?;
            target.FillRoundedRectangle(&rounded(track, thickness / 2.0), &empty);
            let length = thickness + (track.bottom - track.top - thickness) * scene.level;
            let level = rect(track.left, track.bottom - length, track.right, track.bottom);
            let base = mix(rgba(fill, 1.0), grey, scene.mute);
            let light = mix(mix(rgba(fill, 1.0), rgba(ink, 1.0), 0.45), grey, scene.mute);
            let stops = target.CreateGradientStopCollection(
                &[
                    D2D1_GRADIENT_STOP {
                        position: 0.0,
                        color: light,
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.0,
                        color: base,
                    },
                ],
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            )?;
            let gradient = target.CreateLinearGradientBrush(
                &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: Vector2 {
                        X: 0.0,
                        Y: level.top,
                    },
                    endPoint: Vector2 {
                        X: 0.0,
                        Y: level.bottom,
                    },
                },
                None,
                &stops,
            )?;
            target.FillRoundedRectangle(&rounded(level, thickness / 2.0), &gradient);
            // Which channel: its icon in the fill and its name in its color.
            let center_x = (track.left + track.right) / 2.0;
            self.icon(
                icon,
                WHITE,
                1.0 - 0.4 * scene.mute,
                center_x,
                track.bottom - thickness / 2.0,
                22.0,
            )?;
            let caption = rect(
                bounds.left + 6.0,
                bounds.bottom - 38.0,
                bounds.right - 6.0,
                bounds.bottom - 14.0,
            );
            self.text(target, name, &self.name, caption, rgba(ink, 1.0))?;
        }
        Ok(())
    }

    unsafe fn dictation(
        &mut self,
        target: &ID2D1RenderTarget,
        scene: &Scene,
        bounds: D2D_RECT_F,
        view: &DictationView,
    ) -> Result<()> {
        use macafon_ipc::DictationPopupLook;
        let listening = view.state == DictationState::Listening;
        let transcribing = view.state == DictationState::Transcribing;
        let danger = view.state == DictationState::Error;
        let caption = match view.state {
            DictationState::Cancelled => "",
            DictationState::Listening => "",
            DictationState::Transcribing => "TRANSCRIBING",
            DictationState::Done => "TYPED",
            DictationState::Copied => "COPIED · CTRL+V",
            DictationState::NoSpeech => "NO SPEECH",
            DictationState::Error => "DICTATION ERROR",
        };
        // SAFETY: this method is called between BeginDraw and EndDraw on the popup thread.
        unsafe {
            let ink = target.CreateSolidColorBrush(&rgba(INK, 1.0), None)?;
            let red = target.CreateSolidColorBrush(&rgba(DANGER, 1.0), None)?;
            if view.look == DictationPopupLook::Flow {
                let radius = (bounds.bottom - bounds.top) / 2.0;
                self.card(target, bounds, radius)?;
                let fill = target.CreateSolidColorBrush(&rgba(0x0b0b0c, 0.92), None)?;
                target.FillRoundedRectangle(&rounded(bounds, radius), &fill);
                if danger {
                    target.DrawRoundedRectangle(&rounded(bounds, radius), &red, 1.0, None);
                }
                let center_y = (bounds.top + bounds.bottom) / 2.0;
                if listening {
                    // The last 30 seconds before the limit: the bars turn amber.
                    let bars = if view.elapsed_s >= 270 {
                        target.CreateSolidColorBrush(&rgba(0xffc14d, 1.0), None)?
                    } else {
                        ink.clone()
                    };
                    let left = if view.hands_free {
                        bounds.left + 14.0
                    } else {
                        (bounds.left + bounds.right - FLOW_BARS_W) / 2.0
                    };
                    for n in 0..15 {
                        let energy = view.bands[n.min(14 - n)];
                        let mirrored = 1.0 - ((n as f32 - 7.0) / 8.0).abs() * 0.25;
                        let height = 3.0 + energy.clamp(0.0, 1.0) * 18.0 * mirrored;
                        let x = left + n as f32 * 6.0;
                        target.FillRoundedRectangle(
                            &rounded(
                                rect(x, center_y - height / 2.0, x + 3.0, center_y + height / 2.0),
                                1.5,
                            ),
                            &bars,
                        );
                    }
                } else if transcribing {
                    for n in 0..3 {
                        let phase = (scene.shimmer * std::f32::consts::TAU / 0.9 - n as f32 * 0.8)
                            .sin()
                            * 0.5
                            + 0.5;
                        let brush =
                            target.CreateSolidColorBrush(&rgba(INK, 0.3 + 0.7 * phase), None)?;
                        target.FillEllipse(
                            &D2D1_ELLIPSE {
                                point: Vector2 {
                                    X: (bounds.left + bounds.right) / 2.0 + (n as f32 - 1.0) * 14.0,
                                    Y: center_y,
                                },
                                radiusX: 3.0,
                                radiusY: 3.0,
                            },
                            &brush,
                        );
                    }
                } else if matches!(view.state, DictationState::Done | DictationState::Copied) {
                    let x = bounds.left
                        + if view.state == DictationState::Done {
                            10.0
                        } else {
                            18.0
                        };
                    target.DrawLine(
                        Vector2 { X: x, Y: center_y },
                        Vector2 {
                            X: x + 5.0,
                            Y: center_y + 5.0,
                        },
                        &ink,
                        2.0,
                        None,
                    );
                    target.DrawLine(
                        Vector2 {
                            X: x + 5.0,
                            Y: center_y + 5.0,
                        },
                        Vector2 {
                            X: x + 15.0,
                            Y: center_y - 7.0,
                        },
                        &ink,
                        2.0,
                        None,
                    );
                    if view.state == DictationState::Copied {
                        self.text(
                            target,
                            "Copied",
                            &self.dictation_body,
                            rect(x + 26.0, bounds.top, bounds.right - 16.0, bounds.bottom),
                            rgba(INK, 1.0),
                        )?;
                    }
                } else {
                    self.dictation_hud_text(
                        target,
                        caption,
                        rect(
                            bounds.left + 20.0,
                            bounds.top,
                            bounds.right - 20.0,
                            bounds.bottom,
                        ),
                        rgba(if danger { DANGER } else { INK }, 1.0),
                        false,
                    )?;
                }
            } else {
                self.analog(target, scene, bounds, view, caption)?;
            }
            if view.hands_free && listening {
                let y = if view.look == DictationPopupLook::Flow {
                    (bounds.top + bounds.bottom) / 2.0 - 3.0
                } else {
                    bounds.top + 8.0
                };
                target.FillRectangle(
                    &rect(bounds.right - 39.0, y, bounds.right - 33.0, y + 6.0),
                    &ink,
                );
                target.DrawLine(
                    Vector2 {
                        X: bounds.right - 21.0,
                        Y: y,
                    },
                    Vector2 {
                        X: bounds.right - 15.0,
                        Y: y + 6.0,
                    },
                    &ink,
                    1.0,
                    None,
                );
                target.DrawLine(
                    Vector2 {
                        X: bounds.right - 15.0,
                        Y: y,
                    },
                    Vector2 {
                        X: bounds.right - 21.0,
                        Y: y + 6.0,
                    },
                    &ink,
                    1.0,
                    None,
                );
            }
        }
        Ok(())
    }

    /// The Analog look: a thin arc with the red zone, a white needle with VU ballistics and
    /// its slowly falling peak ghost, a lamp glow that breathes with the voice; while
    /// transcribing the needle sways and a light runs along the arc.
    unsafe fn analog(
        &self,
        target: &ID2D1RenderTarget,
        scene: &Scene,
        bounds: D2D_RECT_F,
        view: &DictationView,
        caption: &str,
    ) -> Result<()> {
        let listening = view.state == DictationState::Listening;
        let transcribing = view.state == DictationState::Transcribing;
        let danger = view.state == DictationState::Error;
        let done = matches!(view.state, DictationState::Done | DictationState::Copied);
        // SAFETY: called between BeginDraw and EndDraw on the popup thread.
        unsafe {
            let brush = |hex: u32, a: f32| target.CreateSolidColorBrush(&rgba(hex, a), None);
            target.FillRectangle(&bounds, &brush(BG_RAISED, 1.0)?);
            let line = brush(INK, 0.22)?;
            target.DrawRectangle(&bounds, &line, 1.0, None);
            let tick = brush(INK, 0.45)?;
            for x in [bounds.left, bounds.right] {
                for y in [bounds.top, bounds.bottom] {
                    target.DrawLine(
                        Vector2 { X: x - 4.0, Y: y },
                        Vector2 { X: x + 4.0, Y: y },
                        &tick,
                        1.0,
                        None,
                    );
                    target.DrawLine(
                        Vector2 { X: x, Y: y - 4.0 },
                        Vector2 { X: x, Y: y + 4.0 },
                        &tick,
                        1.0,
                        None,
                    );
                }
            }
            let center = Vector2 {
                X: (bounds.left + bounds.right) / 2.0,
                Y: bounds.top + 42.0,
            };
            let radius = 32.0;
            // −50°…+50° from vertical.
            let point = |travel: f32, r: f32| {
                let a = (-50.0 + 100.0 * travel).to_radians();
                Vector2 {
                    X: center.X + a.sin() * r,
                    Y: center.Y - a.cos() * r,
                }
            };
            let level = vu_travel(scene.vu).clamp(0.0, 1.0);
            // The lamp: an amber glow under the arc that breathes with the voice.
            if listening || transcribing {
                target.PushAxisAlignedClip(&bounds, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                let strength = if listening { 0.05 + 0.16 * level } else { 0.04 };
                for (r, a) in [(38.0, 0.35), (26.0, 0.6), (15.0, 1.0)] {
                    target.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: center,
                            radiusX: r,
                            radiusY: r * 0.8,
                        },
                        &brush(0xf2a516, strength * a)?,
                    );
                }
                target.PopAxisAlignedClip();
            }
            let red_from = vu_travel(1.0);
            let segments = 48;
            let faint = brush(INK, 0.38)?;
            let red = brush(DANGER, 1.0)?;
            for n in 0..segments {
                let (a, b) = (n as f32 / segments as f32, (n + 1) as f32 / segments as f32);
                let hot = a >= red_from - 0.001;
                target.DrawLine(
                    point(a, radius),
                    point(b, radius),
                    if hot { &red } else { &faint },
                    if hot { 2.0 } else { 1.0 },
                    None,
                );
            }
            for vu in [-20.0f32, -10.0, -5.0, 0.0, 3.0] {
                let travel = vu_travel(10f32.powf(vu / 20.0));
                target.DrawLine(
                    point(travel, radius - 4.0),
                    point(travel, radius),
                    if vu >= 0.0 { &red } else { &faint },
                    1.0,
                    None,
                );
            }
            // Transcribing: a light runs along the arc.
            if transcribing {
                let head = (scene.shimmer * 0.9).fract();
                for n in 0..6 {
                    let at = head - n as f32 * 0.035;
                    if at < 0.0 {
                        continue;
                    }
                    let glow = brush(0xffc14d, 1.0 - n as f32 / 6.0)?;
                    let p = point(at, radius);
                    target.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: p,
                            radiusX: 1.6,
                            radiusY: 1.6,
                        },
                        &glow,
                    );
                }
            }
            self.text(
                target,
                "VU",
                &self.dictation_serif,
                rect(
                    center.X - 30.0,
                    center.Y - 16.0,
                    center.X - 10.0,
                    center.Y - 1.0,
                ),
                rgba(INK, 0.35),
            )?;
            // Peak ghost, then the needle.
            let needle_travel = if transcribing {
                0.18 + 0.1 * (scene.shimmer * 2.2).sin()
            } else {
                vu_travel(scene.vu)
            };
            if listening && scene.vu_peak > needle_travel + 0.02 {
                target.DrawLine(
                    center,
                    point(scene.vu_peak, radius - 3.0),
                    &brush(INK, 0.22)?,
                    1.0,
                    None,
                );
            }
            let white = brush(WHITE, if done { 0.7 } else { 1.0 })?;
            target.DrawLine(
                center,
                point(needle_travel, radius - 3.0),
                &white,
                1.5,
                None,
            );
            target.FillEllipse(
                &D2D1_ELLIPSE {
                    point: center,
                    radiusX: 2.5,
                    radiusY: 2.5,
                },
                &white,
            );
            // The peak lamp lights on clipping.
            if listening && view.rms_dbfs > -7.0 {
                target.FillRectangle(
                    &rect(
                        bounds.right - 12.0,
                        bounds.top + 8.0,
                        bounds.right - 8.0,
                        bounds.top + 12.0,
                    ),
                    &red,
                );
            }
            // HUD line: a blinking REC dot while listening, the state otherwise; no timer and
            // no language, only the voice.
            let hud_top = bounds.top + 47.0;
            if listening {
                let on = (scene.shimmer.fract()) < 0.6;
                let dot = if view.elapsed_s >= 270 {
                    0xffc14d
                } else {
                    DANGER
                };
                target.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: Vector2 {
                            X: bounds.left + 10.5,
                            Y: hud_top + 5.5,
                        },
                        radiusX: 2.0,
                        radiusY: 2.0,
                    },
                    &brush(dot, if on { 1.0 } else { 0.25 })?,
                );
            } else {
                let label = match view.state {
                    DictationState::Done => format!("TYPED · {}", view.words),
                    _ => caption.to_owned(),
                };
                self.dictation_hud_text(
                    target,
                    &label,
                    rect(
                        bounds.left + 8.0,
                        hud_top,
                        bounds.right - 8.0,
                        bounds.bottom - 2.0,
                    ),
                    rgba(if danger { DANGER } else { INK }, 0.85),
                    false,
                )?;
            }
        }
        Ok(())
    }

    unsafe fn status(
        &mut self,
        target: &ID2D1RenderTarget,
        scene: &Scene,
        bounds: D2D_RECT_F,
        radius: f32,
    ) -> Result<()> {
        let Osd::Status {
            icon, title, value, ..
        } = scene.view
        else {
            return Ok(());
        };
        // SAFETY: called between BeginDraw and EndDraw.
        unsafe {
            self.card(target, bounds, radius)?;
            let center = Vector2 {
                X: bounds.left + STATUS_H / 2.0,
                Y: (bounds.top + bounds.bottom) / 2.0,
            };
            let badge = target.CreateSolidColorBrush(
                &mix(rgba(ACCENT, 1.0), rgba(DANGER, 1.0), scene.alert),
                None,
            )?;
            target.FillEllipse(
                &D2D1_ELLIPSE {
                    point: center,
                    radiusX: 22.0,
                    radiusY: 22.0,
                },
                &badge,
            );
            let id = match icon {
                StatusIcon::Mic => "mic",
                StatusIcon::MicOff => "mic-off",
                StatusIcon::Processing => "processing",
                StatusIcon::Preset => "preset",
            };
            self.icon(id, WHITE, 1.0, center.X, center.Y, 22.0)?;
            let left = bounds.left + STATUS_H;
            self.text(
                target,
                title,
                &self.title,
                rect(
                    left,
                    bounds.top + 11.0,
                    bounds.right - 20.0,
                    bounds.top + 31.0,
                ),
                rgba(INK, 0.6),
            )?;
            self.text(
                target,
                value,
                &self.value,
                rect(
                    left,
                    bounds.top + 28.0,
                    bounds.right - 20.0,
                    bounds.top + 54.0,
                ),
                rgba(INK, 1.0),
            )?;
        }
        Ok(())
    }

    /// Windows 11-like: the channel's icon, its name over a thin slider with a round thumb, the
    /// percentage at the end.
    unsafe fn bar(
        &mut self,
        target: &ID2D1RenderTarget,
        scene: &Scene,
        bounds: D2D_RECT_F,
        name: &str,
        color: &str,
        icon: &str,
    ) -> Result<()> {
        let right = scene.stretch.max(0.0) * STRETCH / KICK * 4.0;
        let left = (-scene.stretch).max(0.0) * STRETCH / KICK * 4.0;
        let bounds = rect(
            bounds.left - left,
            bounds.top,
            bounds.right + right,
            bounds.bottom,
        );
        let (fill, ink) = channel_colors(color);
        let grey = rgba(INK, 0.32);
        let middle = (bounds.top + bounds.bottom) / 2.0;
        // SAFETY: called between BeginDraw and EndDraw.
        unsafe {
            self.card(target, bounds, BAR_RADIUS)?;
            self.icon(
                icon,
                ink,
                1.0 - 0.5 * scene.mute,
                bounds.left + 30.0,
                middle,
                22.0,
            )?;
            let (start, end) = (bounds.left + 56.0, bounds.right - 74.0);
            let caption = rect(start, bounds.top + 9.0, end, bounds.top + 27.0);
            self.text(target, name, &self.label, caption, rgba(ink, 1.0))?;
            let y = bounds.top + 40.0;
            let empty = target.CreateSolidColorBrush(&rgba(INK, 0.18), None)?;
            target.FillRoundedRectangle(&rounded(rect(start, y - 2.0, end, y + 2.0), 2.0), &empty);
            let x = start + (end - start) * scene.level;
            let filled =
                target.CreateSolidColorBrush(&mix(rgba(fill, 1.0), grey, scene.mute), None)?;
            target.FillRoundedRectangle(
                &rounded(rect(start, y - 2.0, x.max(start + 4.0), y + 2.0), 2.0),
                &filled,
            );
            let ring = target
                .CreateSolidColorBrush(&mix(rgba(BG_RAISED, 1.0), rgba(INK, 1.0), 0.22), None)?;
            let thumb = |radius| D2D1_ELLIPSE {
                point: Vector2 { X: x, Y: y },
                radiusX: radius,
                radiusY: radius,
            };
            target.FillEllipse(&thumb(10.0), &ring);
            target.FillEllipse(&thumb(6.0), &filled);
            let number = rect(
                bounds.right - 60.0,
                bounds.top,
                bounds.right - 10.0,
                bounds.bottom,
            );
            let percent = format!("{:.0}%", scene.level * 100.0);
            self.text(
                target,
                &percent,
                &self.percent,
                number,
                rgba(INK, 1.0 - scene.mute),
            )?;
            self.text(
                target,
                "Muted",
                &self.percent,
                number,
                rgba(DANGER, scene.mute),
            )?;
        }
        Ok(())
    }

    /// A Lucide icon centered on `(x, y)` in panel coordinates.
    unsafe fn icon(
        &mut self,
        id: &str,
        color: u32,
        opacity: f32,
        x: f32,
        y: f32,
        size: f32,
    ) -> Result<()> {
        let Some(context) = self.context.clone() else {
            return Ok(());
        };
        let Some(&(key, body)) = ICONS
            .iter()
            .find(|(known, _)| *known == id)
            .or(ICONS.first())
        else {
            return Ok(());
        };
        // Quantized so a fade reuses a handful of documents.
        let alpha = (opacity.clamp(0.0, 1.0) * 16.0).round() as u32;
        let cache = (key, color << 8 | alpha, size as u32);
        // SAFETY: called between BeginDraw and EndDraw; the stream is read during the call.
        unsafe {
            if !self.icons.contains_key(&cache) {
                let svg = format!(
                    r##"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 24 24" fill="none" stroke="#{color:06x}" stroke-opacity="{}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"##,
                    alpha as f32 / 16.0
                );
                let stream = SHCreateMemStream(Some(svg.as_bytes()))
                    .ok_or_else(windows::core::Error::from_thread)?;
                let document = context.CreateSvgDocument(
                    &stream,
                    D2D_SIZE_F {
                        width: size,
                        height: size,
                    },
                )?;
                if self.icons.len() > 64 {
                    self.icons.clear();
                }
                self.icons.insert(cache, document);
            }
            let p = &self.panel;
            let (left, top) = (x - size / 2.0, y - size / 2.0);
            context.SetTransform(&Matrix3x2 {
                M31: p.M11 * left + p.M31,
                M32: p.M22 * top + p.M32,
                ..*p
            });
            if let Some(document) = self.icons.get(&cache) {
                context.DrawSvgDocument(document);
            }
            context.SetTransform(p);
        }
        Ok(())
    }

    /// Copies the frame into the DIB and hands it to the layered window.
    fn present(&self, window: HWND, origin: POINT, opacity: f32) -> Result<()> {
        let Some(bitmap) = &self.bitmap else {
            return Ok(());
        };
        let stride = self.width * 4;
        // SAFETY: `bits` is the live DIB of `width × height` 32-bit pixels selected into `dc`.
        unsafe {
            let pixels = std::slice::from_raw_parts_mut(self.bits, (stride * self.height) as usize);
            bitmap.CopyPixels(std::ptr::null(), stride, pixels)?;
            let size = SIZE {
                cx: self.width as i32,
                cy: self.height as i32,
            };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: (opacity * 255.0).round() as u8,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(
                window,
                None,
                Some(&origin),
                Some(&size),
                Some(self.dc),
                Some(&POINT::default()),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        }
    }

    /// Previews: the current frame as a PNG.
    #[cfg(test)]
    fn save_png(&self, path: &std::path::Path) -> Result<()> {
        let Some(bitmap) = &self.bitmap else {
            return Ok(());
        };
        // SAFETY: COM calls on this thread with live arguments.
        unsafe {
            let stream = self.wic.CreateStream()?;
            stream.InitializeFromFilename(&HSTRING::from(path.as_os_str()), GENERIC_WRITE.0)?;
            let encoder = self
                .wic
                .CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
            encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
            let mut frame = None;
            encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
            let frame = frame.ok_or_else(windows::core::Error::from_thread)?;
            frame.Initialize(None)?;
            frame.SetSize(self.width, self.height)?;
            let mut format = GUID_WICPixelFormat32bppBGRA;
            frame.SetPixelFormat(&mut format)?;
            frame.WriteSource(bitmap, std::ptr::null())?;
            frame.Commit()?;
            encoder.Commit()
        }
    }
}

impl Drop for Painter {
    fn drop(&mut self) {
        // SAFETY: GDI objects owned by this painter.
        unsafe {
            let _ = DeleteDC(self.dc);
            if !self.dib.is_invalid() {
                let _ = DeleteObject(self.dib.into());
            }
        }
    }
}

fn format(
    dwrite: &IDWriteFactory,
    family: PCWSTR,
    weight: DWRITE_FONT_WEIGHT,
    size: f32,
    centered: bool,
) -> Result<IDWriteTextFormat> {
    // SAFETY: DirectWrite copies the family name.
    unsafe {
        let format = dwrite.CreateTextFormat(
            family,
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-us"),
        )?;
        format.SetTextAlignment(if centered {
            DWRITE_TEXT_ALIGNMENT_CENTER
        } else {
            DWRITE_TEXT_ALIGNMENT_LEADING
        })?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        let ellipsis = dwrite.CreateEllipsisTrimmingSign(&format)?;
        format.SetTrimming(
            &DWRITE_TRIMMING {
                granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                delimiter: 0,
                delimiterCount: 0,
            },
            &ellipsis,
        )?;
        Ok(format)
    }
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

    #[test]
    fn springs_bounce_once_and_settle_quickly() {
        let mut enter = Spring {
            target: 1.0,
            ..Spring::default()
        };
        let mut peak: f32 = 0.0;
        let mut elapsed = 0.0;
        while !enter.settled() && elapsed < 2.0 {
            enter.step(1.0 / 60.0, ENTER.0, ENTER.1);
            peak = peak.max(enter.value);
            elapsed += 1.0 / 60.0;
        }
        assert!(peak > 1.02 && peak < 1.15, "a light overshoot, got {peak}");
        assert!(elapsed < 0.6, "settles in {elapsed} s");
        let mut level = Spring {
            target: 1.0,
            ..Spring::default()
        };
        for _ in 0..12 {
            level.step(1.0 / 60.0, LEVEL.0, LEVEL.1);
        }
        assert!(
            (level.value - 1.0).abs() < 0.03,
            "a volume step lands in 200 ms"
        );
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
