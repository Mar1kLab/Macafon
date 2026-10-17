//! Windows popup: a layered, click-through, topmost window owned by its own thread. Direct2D draws
//! each frame into a WIC bitmap, `UpdateLayeredWindow` shows it; springs drive every motion and the
//! loop paces itself on `DwmFlush` while anything moves.
use super::icons::ICONS;
use super::{Osd, StatusIcon};
use macafon_config::{OsdConfig, OsdEdge, OsdStyle};
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
const LEVEL: (f32, f32) = (6.0, 0.9);
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
                STATUS_H + self.pad + SHADOW,
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
        }
    }

    fn receive(&mut self, osd: Osd, setup: OsdConfig, now: Instant) {
        let same = match (&self.view, &osd) {
            (Some(Osd::Volume { name: a, .. }), Osd::Volume { name: b, .. }) => a == b,
            (Some(Osd::Status { title: a, .. }), Osd::Status { title: b, .. }) => a == b,
            _ => false,
        };
        // Settings changing the setup moves a visible popup right away.
        if !self.visible || setup != self.layout.setup {
            if let Err(error) = self.place(setup) {
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
            Osd::Volume {
                volume_pct, muted, ..
            } => {
                let target = (volume_pct / 100.0).clamp(0.0, 1.0);
                // Pressing on at a limit stretches the slider like Android's overscroll.
                if same && target == self.level.target && (target == 0.0 || target == 1.0) {
                    self.stretch.velocity += if target == 1.0 { KICK } else { -KICK };
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
                let text = self
                    .painter
                    .text_width(title, &self.painter.title)
                    .max(self.painter.text_width(value, &self.painter.value));
                self.status_width = (64.0 + text + 24.0).clamp(STATUS_MIN_W, STATUS_MAX_W);
            }
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
        if !self.exiting && now - self.changed_at >= HOLD {
            self.exiting = true;
            self.enter.target = 0.0;
            if !animations_enabled() {
                self.enter.snap(0.0);
            }
        }
        let (frequency, damping) = if self.exiting { EXIT } else { ENTER };
        self.enter.step(dt, frequency, damping);
        self.level.step(dt, LEVEL.0, LEVEL.1);
        self.mute.step(dt, FADE.0, FADE.1);
        self.alert.step(dt, FADE.0, FADE.1);
        self.stretch.step(dt, BOUNCE.0, BOUNCE.1);
        if self.exiting && self.enter.value <= 0.01 {
            self.hide();
            return false;
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
        let left = HOLD.saturating_sub(now - self.changed_at);
        u32::try_from(left.as_millis()).unwrap_or(u32::MAX).max(1)
    }

    fn hide(&mut self) {
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
    fn place(&mut self, setup: OsdConfig) -> Result<()> {
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
            let layout = Layout {
                setup,
                pad: MARGIN + (offset as f32 / scale).min(SLIDE + SHADOW),
            };
            self.painter.resize(scale, layout.window())?;
            let px = |dip: f32| (dip * scale).round() as i32;
            let (w, h) = (self.painter.width as i32, self.painter.height as i32);
            // The panel's edge lands `MARGIN + offset` from the work area's edge.
            let inset = offset + px(MARGIN) - px(layout.pad);
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
        };
        self.painter.paint(&scene, self.layout, None)?;
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
    percent: IDWriteTextFormat,
    name: IDWriteTextFormat,
    label: IDWriteTextFormat,
    title: IDWriteTextFormat,
    value: IDWriteTextFormat,
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
            let percent = format(&dwrite, display, DWRITE_FONT_WEIGHT_SEMI_BOLD, 15.0, true)?;
            let name = format(&dwrite, text, DWRITE_FONT_WEIGHT_SEMI_BOLD, 12.0, true)?;
            let label = format(&dwrite, text, DWRITE_FONT_WEIGHT_SEMI_BOLD, 12.0, false)?;
            let title = format(&dwrite, text, DWRITE_FONT_WEIGHT_NORMAL, 12.0, false)?;
            let value = format(&dwrite, display, DWRITE_FONT_WEIGHT_SEMI_BOLD, 16.0, false)?;
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
                percent,
                name,
                label,
                title,
                value,
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

    fn paint(&mut self, scene: &Scene, layout: Layout, backdrop: Option<u32>) -> Result<()> {
        let Some(target) = self.target.clone() else {
            return Ok(());
        };
        let horizontal = layout.setup.style == OsdStyle::Horizontal;
        let panel = match scene.view {
            Osd::Volume { .. } if horizontal => (BAR_W, BAR_H),
            Osd::Volume { .. } => (VOLUME_W, VOLUME_H),
            Osd::Status { .. } => (scene.status_width, STATUS_H),
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
            let drawn = match scene.view {
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
            };
            let ended = target.EndDraw(None, None);
            drawn.and(ended)
        }
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
        for (style_name, style, edge) in layouts {
            let layout = Layout {
                setup: OsdConfig {
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
                    Osd::Status { alert, .. } => (0.0, 0.0, f32::from(u8::from(*alert))),
                };
                let status_width = match view {
                    Osd::Status { title, value, .. } => (64.0
                        + painter
                            .text_width(title, &painter.title)
                            .max(painter.text_width(value, &painter.value))
                        + 24.0)
                        .clamp(STATUS_MIN_W, STATUS_MAX_W),
                    Osd::Volume { .. } => STATUS_MIN_W,
                };
                let scene = Scene {
                    view,
                    enter: 1.0,
                    level,
                    mute,
                    alert,
                    stretch: *stretch,
                    status_width,
                };
                painter
                    .paint(&scene, layout, Some(0x3a4a5c))
                    .expect("paint");
                painter
                    .save_png(&directory.join(format!("2026-10-17-osd-{style_name}-{label}.png")))
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
                };
                painter
                    .paint(&scene, layout, Some(0x3a4a5c))
                    .expect("paint");
                painter
                    .save_png(
                        &directory.join(format!("2026-10-17-osd-{style_name}-enter-{frame}.png")),
                    )
                    .expect("png");
                enter.step(1.0 / 30.0, ENTER.0, ENTER.1);
            }
        }
    }
}
