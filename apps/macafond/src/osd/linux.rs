//! Linux popup (HK-006, §9.9.3): the Windows popup's looks and motion in a wlr-layer-shell
//! overlay surface owned by its own thread. The surface never takes the keyboard, is
//! click-through (except hands-free dictation's stop and cancel) and sits on the configured
//! edge of the active output (the compositor's choice) or of the primary one. The daemon draws
//! it, so it works with the UI closed. Without layer-shell (GNOME, X11) `show` declines and the
//! caller falls back to a replaceable notification.
use super::Osd;
use super::dictation::State as DictationState;
use super::motion::{Layout, Motion, Step, edge_margin};
use macafon_config::{OsdConfig, OsdEdge};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{
        Shm, ShmHandler,
        slot::{Buffer, SlotPool},
    },
};
use std::io::{Read, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::net::UnixStream;
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    globals::{GlobalList, registry_queue_init},
    protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};
use wayland_protocols_plasma::primary_output::v1::client::kde_primary_output_v1::{
    self, KdePrimaryOutputV1,
};

mod painter;
mod text;
use painter::Painter;

type Message = (Osd, OsdConfig);

/// The popup thread, started on first use and again after it ended (the compositor went
/// away, e.g. a restarted session); a failed start is retried after `RETRY`.
struct Slot {
    handle: Option<Handle>,
    retry_at: Option<Instant>,
}

struct Handle {
    sender: mpsc::Sender<Message>,
    /// One byte wakes the thread's `poll`.
    wake: UnixStream,
}

static POPUP: Mutex<Slot> = Mutex::new(Slot {
    handle: None,
    retry_at: None,
});
const RETRY: Duration = Duration::from_secs(30);
/// Longest wait for a frame callback: a compositor may hold them for a hidden output.
const FRAME_TIMEOUT: Duration = Duration::from_millis(100);
const BTN_LEFT: u32 = 0x110;

/// Whether the popup can show here: a Wayland session whose compositor has layer-shell.
pub(super) fn supported() -> bool {
    let mut slot = POPUP.lock().unwrap_or_else(|error| error.into_inner());
    started(&mut slot).is_some()
}

/// Shows `osd`, or hands it back when there is no layer-shell popup to show it in.
pub(super) fn show(osd: Osd, setup: OsdConfig) -> Option<Osd> {
    let mut slot = POPUP.lock().unwrap_or_else(|error| error.into_inner());
    let mut message = (osd, setup.normalized());
    // A second try only after the thread ended: start it once more.
    for _ in 0..2 {
        let Some(handle) = started(&mut slot) else {
            return Some(message.0);
        };
        match handle.sender.send(message) {
            Ok(()) => {
                // A full socket already holds a pending wake-up.
                let _ = (&handle.wake).write(&[1]);
                return None;
            }
            Err(mpsc::SendError(back)) => message = back,
        }
        slot.handle = None;
        slot.retry_at = None;
    }
    Some(message.0)
}

fn started(slot: &mut Slot) -> Option<&Handle> {
    if slot.handle.is_none() && slot.retry_at.is_none_or(|at| Instant::now() >= at) {
        slot.handle = spawn();
        slot.retry_at = slot.handle.is_none().then(|| Instant::now() + RETRY);
    }
    slot.handle.as_ref()
}

fn spawn() -> Option<Handle> {
    // Without a Wayland display there is nothing to try (X11, a headless service).
    std::env::var_os("WAYLAND_DISPLAY")?;
    let (sender, receiver) = mpsc::channel();
    let (wake, woken) = UnixStream::pair().ok()?;
    wake.set_nonblocking(true).ok()?;
    woken.set_nonblocking(true).ok()?;
    let (ready, started) = mpsc::sync_channel(1);
    let spawned = std::thread::Builder::new()
        .name("macafon-osd".into())
        .spawn(move || {
            if let Err(error) = run(&receiver, woken, &ready) {
                tracing::info!(%error, "Layer-shell hotkey popup unavailable; using notifications");
                let _ = ready.try_send(false);
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "Cannot start hotkey popup thread");
        return None;
    }
    started
        .recv_timeout(Duration::from_secs(3))
        .ok()?
        .then_some(Handle { sender, wake })
}

fn run(
    receiver: &mpsc::Receiver<Message>,
    mut woken: UnixStream,
    ready: &mpsc::SyncSender<bool>,
) -> anyhow::Result<()> {
    let connection = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<Popup>(&connection)?;
    let qh = queue.handle();
    let mut popup = Popup::new(&globals, &qh)?;
    // Outputs, seats and the primary output arrive with the first roundtrips.
    queue.roundtrip(&mut popup)?;
    queue.roundtrip(&mut popup)?;
    let _ = ready.send(true);
    tracing::info!(
        fractional = popup.fractional.is_some(),
        "Layer-shell hotkey popup ready"
    );
    let mut moving = false;
    let mut last = Instant::now();
    loop {
        let now = Instant::now();
        let mut dirty = false;
        loop {
            match receiver.try_recv() {
                Ok((osd, setup)) => {
                    popup.receive(osd, setup, now, &qh);
                    dirty = true;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }
        // The first frame of a motion starts from rest, not from the idle wait.
        let dt = if moving {
            (now - last).as_secs_f32().min(1.0 / 30.0)
        } else {
            0.0
        };
        last = now;
        moving = false;
        if popup.motion.visible {
            let animations = popup.animations;
            match popup.motion.advance(now, dt, || animations) {
                Step::Moving => moving = true,
                Step::Resting => {}
                Step::Hidden => popup.close(),
            }
        }
        if popup
            .frame_pending
            .is_some_and(|at| now - at > FRAME_TIMEOUT)
        {
            popup.frame_pending = None;
        }
        if popup.motion.visible && (moving || dirty || popup.redraw) {
            popup.frame(&qh);
        }
        connection.flush()?;
        let timeout = if moving || popup.redraw {
            // Paced by the compositor's frame callbacks.
            Some(if popup.frame_pending.is_some() {
                FRAME_TIMEOUT
            } else {
                Duration::from_millis(4)
            })
        } else {
            popup.motion.wait(Instant::now())
        };
        wait(&mut queue, &mut popup, &mut woken, timeout)?;
    }
}

/// Sleeps until a Wayland event, a message or `timeout` (`None`: no limit).
fn wait(
    queue: &mut wayland_client::EventQueue<Popup>,
    popup: &mut Popup,
    woken: &mut UnixStream,
    timeout: Option<Duration>,
) -> anyhow::Result<()> {
    queue.dispatch_pending(popup)?;
    if let Some(guard) = queue.prepare_read() {
        let mut fds = [
            libc::pollfd {
                fd: guard.connection_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: woken.as_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let timeout_ms = timeout.map_or(-1, |left| {
            i32::try_from(left.as_millis()).unwrap_or(i32::MAX).max(1)
        });
        // SAFETY: two live pollfds for the duration of the call, on the popup's own thread.
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout_ms) };
        if ready > 0 && fds[0].revents != 0 {
            guard.read()?;
        }
        if ready > 0 && fds[1].revents != 0 {
            let mut bytes = [0u8; 64];
            while matches!(woken.read(&mut bytes), Ok(n) if n > 0) {}
        }
    }
    queue.dispatch_pending(popup)?;
    Ok(())
}

/// KDE's "animation speed" at its slowest end turns animations off (the Windows popup follows
/// "Animation effects" the same way). Read once per appearance, never per frame.
fn animations_enabled() -> bool {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(".config"))
        });
    let Some(text) = config.and_then(|dir| std::fs::read_to_string(dir.join("kdeglobals")).ok())
    else {
        return true;
    };
    !kde_animations_off(&text)
}

fn kde_animations_off(kdeglobals: &str) -> bool {
    let mut group = "";
    for line in kdeglobals.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            group = name;
        } else if group == "KDE"
            && let Some(value) = line.strip_prefix("AnimationDurationFactor=")
        {
            return value
                .trim()
                .parse::<f32>()
                .is_ok_and(|factor| factor <= 0.0);
        }
    }
    false
}

/// The surface of one appearance and what came with it.
struct Surface {
    layer: LayerSurface,
    fractional: Option<WpFractionalScaleV1>,
    viewport: Option<WpViewport>,
    configured: bool,
    /// Logical size requested from the compositor.
    size: (u32, u32),
    /// Distance from the anchored edge to the window, logical pixels.
    inset: i32,
    /// The pixels last attached.
    buffer: Option<Buffer>,
}

impl Drop for Surface {
    fn drop(&mut self) {
        if let Some(viewport) = self.viewport.take() {
            viewport.destroy();
        }
        if let Some(fractional) = self.fractional.take() {
            fractional.destroy();
        }
    }
}

struct Popup {
    registry: RegistryState,
    outputs: OutputState,
    seats: SeatState,
    compositor: CompositorState,
    shell: LayerShell,
    shm: Shm,
    pool: SlotPool,
    fractional: Option<WpFractionalScaleManagerV1>,
    viewporter: Option<WpViewporter>,
    /// KDE names the primary output; elsewhere it is the one at the layout's origin.
    _primary: Option<KdePrimaryOutputV1>,
    primary_name: Option<String>,
    pointer: Option<wl_pointer::WlPointer>,
    surface: Option<Surface>,
    /// The output the surface was last shown on: the best guess for the next offset.
    entered: Option<wl_output::WlOutput>,
    /// Preferred scale: from wp_fractional_scale (×120) or the integer buffer scale.
    scale_120: u32,
    painter: Painter,
    layout: Layout,
    dictation: bool,
    motion: Motion,
    animations: bool,
    frame_pending: Option<Instant>,
    /// A frame is due though nothing moves (a configure, a new scale).
    redraw: bool,
}

impl Popup {
    fn new(globals: &GlobalList, qh: &QueueHandle<Self>) -> anyhow::Result<Self> {
        let compositor = CompositorState::bind(globals, qh)?;
        let shell = LayerShell::bind(globals, qh)?;
        let shm = Shm::bind(globals, qh)?;
        let pool = SlotPool::new(256 * 1024, &shm)?;
        let painter =
            Painter::new().ok_or_else(|| anyhow::anyhow!("bundled popup fonts unreadable"))?;
        Ok(Self {
            registry: RegistryState::new(globals),
            outputs: OutputState::new(globals, qh),
            seats: SeatState::new(globals, qh),
            compositor,
            shell,
            shm,
            pool,
            fractional: globals.bind(qh, 1..=1, ()).ok(),
            viewporter: globals.bind(qh, 1..=1, ()).ok(),
            _primary: globals.bind(qh, 1..=2, ()).ok(),
            primary_name: None,
            pointer: None,
            surface: None,
            entered: None,
            scale_120: 120,
            painter,
            layout: Layout::new(OsdConfig::default(), edge_margin(false), 0.0),
            dictation: false,
            motion: Motion::new(),
            animations: true,
            frame_pending: None,
            redraw: false,
        })
    }

    fn receive(&mut self, osd: Osd, setup: OsdConfig, now: Instant, qh: &QueueHandle<Self>) {
        if matches!(&osd, Osd::Dictation(v) if v.state == DictationState::Cancelled) {
            self.close();
            self.motion.view = None;
            return;
        }
        let dictation = matches!(osd, Osd::Dictation(_));
        let same = self.motion.continues(&osd);
        // A new appearance is a new surface, so the compositor puts it on the active output;
        // Settings changing the setup moves a visible popup right away.
        if !self.motion.visible
            || self.surface.is_none()
            || setup != self.layout.setup
            || dictation != self.dictation
        {
            if let Err(error) = self.open(setup, dictation, qh) {
                tracing::debug!(%error, "Cannot place hotkey popup");
                return;
            }
        }
        let painter = &self.painter;
        self.motion
            .receive(osd, same, now, self.animations, |title, value| {
                painter.status_width(title, value, setup.look)
            });
    }

    /// The primary output: KDE's, else the one at the layout's origin, else any.
    fn primary(&self) -> Option<wl_output::WlOutput> {
        let outputs: Vec<_> = self.outputs.outputs().collect();
        let info = |output: &wl_output::WlOutput| self.outputs.info(output);
        self.primary_name
            .as_ref()
            .and_then(|name| {
                outputs
                    .iter()
                    .find(|o| info(o).is_some_and(|i| i.name.as_ref() == Some(name)))
            })
            .or_else(|| {
                outputs
                    .iter()
                    .find(|o| info(o).is_some_and(|i| i.location == (0, 0)))
            })
            .or(outputs.first())
            .cloned()
    }

    /// The layout and the window's inset for an output's logical size (`None`: unknown yet).
    fn placement(setup: OsdConfig, dictation: bool, output: Option<(i32, i32)>) -> (Layout, i32) {
        let across = output.map_or(0, |(w, h)| match setup.edge {
            OsdEdge::Left | OsdEdge::Right => w,
            OsdEdge::Top | OsdEdge::Bottom => h,
        });
        let offset = (across as f32 * setup.offset_pct / 100.0).round();
        let margin = edge_margin(dictation);
        let layout = Layout::new(setup, margin, offset);
        // The panel's edge lands `margin + offset` from the work area's edge.
        (layout, (offset + margin - layout.pad).round() as i32)
    }

    fn open(
        &mut self,
        setup: OsdConfig,
        dictation: bool,
        qh: &QueueHandle<Self>,
    ) -> anyhow::Result<()> {
        self.close_surface();
        let output = if setup.primary_monitor {
            self.primary()
        } else {
            // The compositor chooses: the output with the focus or the pointer.
            None
        };
        let guess = output.as_ref().or(self.entered.as_ref());
        let info = guess.and_then(|o| self.outputs.info(o));
        if let Some(info) = &info {
            self.scale_120 = (info.scale_factor.max(1) * 120) as u32;
        }
        let (layout, inset) =
            Self::placement(setup, dictation, info.as_ref().and_then(|i| i.logical_size));
        let size = logical(layout.window());
        let surface = self.compositor.create_surface(qh);
        let fractional = self
            .fractional
            .as_ref()
            .map(|manager| manager.get_fractional_scale(&surface, qh, surface.clone()));
        let viewport = self
            .viewporter
            .as_ref()
            .map(|viewporter| viewporter.get_viewport(&surface, qh, ()));
        // Click-through: an empty input region until hands-free buttons need clicks.
        let empty = Region::new(&self.compositor)?;
        surface.set_input_region(Some(empty.wl_region()));
        let layer = self.shell.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some("macafon-osd"),
            output.as_ref(),
        );
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        // Zero: kept clear of panels, like the Windows popup in the work area.
        layer.set_exclusive_zone(0);
        layer.set_size(size.0, size.1);
        position(&layer, setup.edge, inset);
        layer.commit();
        self.surface = Some(Surface {
            layer,
            fractional,
            viewport,
            configured: false,
            size,
            inset,
            buffer: None,
        });
        self.layout = layout;
        self.dictation = dictation;
        self.animations = animations_enabled();
        self.frame_pending = None;
        Ok(())
    }

    fn close_surface(&mut self) {
        self.surface = None;
        self.frame_pending = None;
        self.redraw = false;
    }

    /// The exit finished or the compositor took the surface away.
    fn close(&mut self) {
        self.close_surface();
        self.motion.hide();
        self.painter.release();
    }

    /// The output the surface entered is known now: correct the offset if it was a guess.
    fn entered(&mut self, output: &wl_output::WlOutput) {
        self.entered = Some(output.clone());
        let info = self.outputs.info(output);
        let (layout, inset) = Self::placement(
            self.layout.setup,
            self.dictation,
            info.as_ref().and_then(|i| i.logical_size),
        );
        let Some(surface) = &mut self.surface else {
            return;
        };
        if self.fractional.is_none() {
            if let Some(info) = &info {
                self.scale_120 = (info.scale_factor.max(1) * 120) as u32;
            }
        }
        if layout == self.layout && inset == surface.inset {
            return;
        }
        let size = logical(layout.window());
        surface.layer.set_size(size.0, size.1);
        position(&surface.layer, layout.setup.edge, inset);
        surface.layer.commit();
        surface.size = size;
        surface.inset = inset;
        self.layout = layout;
        self.redraw = true;
    }

    fn scale(&self) -> f32 {
        if self.viewporter.is_some() {
            self.scale_120 as f32 / 120.0
        } else {
            // Without a viewport only whole buffer scales exist.
            (self.scale_120 as f32 / 120.0).ceil().max(1.0)
        }
    }

    fn frame(&mut self, qh: &QueueHandle<Self>) {
        let scale = self.scale();
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        if !surface.configured || self.frame_pending.is_some() {
            return;
        }
        let Some(scene) = self.motion.scene() else {
            return;
        };
        let window = (surface.size.0 as f32, surface.size.1 as f32);
        if !self.painter.resize(scale, window) {
            return;
        }
        self.painter.paint(&scene, self.layout, None);
        let hands_free = matches!(scene.view, Osd::Dictation(view) if view.hands_free && view.state == DictationState::Listening);
        let panel =
            super::motion::panel_size(scene.view, self.layout.setup.style, scene.status_width);
        let opacity = (self.motion.enter.value * 1.4).clamp(0.0, 1.0);
        let (width, height) = (self.painter.width, self.painter.height);
        let Some(pixels) = self.painter.pixels() else {
            return;
        };
        let stride = width as i32 * 4;
        let reuse = surface
            .buffer
            .as_ref()
            .is_some_and(|b| b.height() == height as i32 && b.stride() == stride);
        if !reuse {
            surface.buffer = None;
        }
        let canvas = match surface
            .buffer
            .as_ref()
            .and_then(|b| b.canvas(&mut self.pool))
        {
            Some(canvas) => canvas,
            None => {
                let created = self.pool.create_buffer(
                    width as i32,
                    height as i32,
                    stride,
                    wl_shm::Format::Argb8888,
                );
                match created {
                    Ok((buffer, canvas)) => {
                        surface.buffer = Some(buffer);
                        canvas
                    }
                    Err(error) => {
                        tracing::debug!(%error, "Hotkey popup buffer unavailable");
                        return;
                    }
                }
            }
        };
        copy_bgra(pixels, canvas, opacity);
        let wl = surface.layer.wl_surface();
        // Hands-free dictation takes clicks on its stop and cancel only.
        if let Ok(region) = Region::new(&self.compositor) {
            if hands_free {
                let (b, _, _) = self.layout.place(self.painter.size, panel);
                let p = self.painter.panel;
                let x = |at: f32| (at * p.sx + p.tx) as i32;
                let top = ((b.top + 1.0) * p.sy + p.ty) as i32;
                let bottom = ((b.top + 26.0) * p.sy + p.ty) as i32;
                region.add(
                    x(b.right - 48.0),
                    top,
                    x(b.right - 7.0) - x(b.right - 48.0),
                    bottom - top,
                );
            }
            wl.set_input_region(Some(region.wl_region()));
        }
        match &surface.viewport {
            Some(viewport) => {
                wl.set_buffer_scale(1);
                viewport.set_destination(surface.size.0 as i32, surface.size.1 as i32);
            }
            None => wl.set_buffer_scale(scale as i32),
        }
        wl.damage_buffer(0, 0, width as i32, height as i32);
        wl.frame(qh, wl.clone());
        if let Some(buffer) = &surface.buffer {
            if let Err(error) = buffer.attach_to(wl) {
                tracing::debug!(%error, "Hotkey popup buffer not attached");
                return;
            }
        }
        surface.layer.commit();
        self.frame_pending = Some(Instant::now());
        self.redraw = false;
    }

    /// A click on hands-free dictation's buttons: stop on ■, cancel on ✕ (as on Windows).
    fn click(&mut self, x: f64, y: f64) {
        let Some(Osd::Dictation(view)) = &self.motion.view else {
            return;
        };
        if !(view.hands_free && view.state == DictationState::Listening) {
            return;
        }
        let panel = super::motion::dictation_size(view);
        let (b, _, _) = self.layout.place(self.painter.size, panel);
        let (x, y) = (x as f32, y as f32);
        if y >= b.top && y <= b.top + 28.0 && x >= b.right - 50.0 && x <= b.right {
            super::dictation_click(x > b.right - 28.0);
        }
    }
}

/// The window in whole logical pixels.
fn logical((width, height): (f32, f32)) -> (u32, u32) {
    (width.ceil() as u32, height.ceil() as u32)
}

/// Anchored to one edge only, the surface is centered along it.
fn position(layer: &LayerSurface, edge: OsdEdge, inset: i32) {
    let (anchor, margin) = match edge {
        OsdEdge::Top => (Anchor::TOP, (inset, 0, 0, 0)),
        OsdEdge::Right => (Anchor::RIGHT, (0, inset, 0, 0)),
        OsdEdge::Bottom => (Anchor::BOTTOM, (0, 0, inset, 0)),
        OsdEdge::Left => (Anchor::LEFT, (0, 0, 0, inset)),
    };
    layer.set_anchor(anchor);
    layer.set_margin(margin.0, margin.1, margin.2, margin.3);
}

/// Premultiplied RGBA into `wl_shm` ARGB8888 (B, G, R, A in memory), faded by `opacity`.
fn copy_bgra(source: &[u8], target: &mut [u8], opacity: f32) {
    let fade = (opacity.clamp(0.0, 1.0) * 255.0).round() as u16;
    let scale = |v: u8| ((u16::from(v) * fade + 127) / 255) as u8;
    for (from, to) in source.chunks_exact(4).zip(target.chunks_exact_mut(4)) {
        if fade == 255 {
            to.copy_from_slice(&[from[2], from[1], from[0], from[3]]);
        } else {
            to.copy_from_slice(&[
                scale(from[2]),
                scale(from[1]),
                scale(from[0]),
                scale(from[3]),
            ]);
        }
    }
}

impl CompositorHandler for Popup {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        factor: i32,
    ) {
        // The integer scale counts only without wp_fractional_scale.
        if self.fractional.is_none() {
            self.scale_120 = (factor.max(1) * 120) as u32;
            self.redraw = true;
        }
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.frame_pending = None;
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        output: &wl_output::WlOutput,
    ) {
        self.entered(output);
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Popup {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if self.entered.as_ref() == Some(&output) {
            self.entered = None;
        }
    }
}

impl LayerShellHandler for Popup {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if self.surface.as_ref().is_some_and(|s| &s.layer == layer) {
            self.close();
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        _: LayerSurfaceConfigure,
        _: u32,
    ) {
        if let Some(surface) = self.surface.as_mut().filter(|s| &s.layer == layer) {
            surface.configured = true;
            self.redraw = true;
        }
    }
}

impl ShmHandler for Popup {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl SeatHandler for Popup {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seats.get_pointer(qh, &seat).ok();
        }
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            if let Some(pointer) = self.pointer.take() {
                pointer.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for Popup {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let ours = self
                .surface
                .as_ref()
                .is_some_and(|s| s.layer.wl_surface() == &event.surface);
            if ours
                && matches!(
                    event.kind,
                    PointerEventKind::Release {
                        button: BTN_LEFT,
                        ..
                    }
                )
            {
                self.click(event.position.0, event.position.1);
            }
        }
    }
}

impl ProvidesRegistryState for Popup {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

impl Dispatch<WpFractionalScaleV1, wl_surface::WlSurface> for Popup {
    fn event(
        popup: &mut Self,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &wl_surface::WlSurface,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            if scale > 0 && scale != popup.scale_120 {
                popup.scale_120 = scale;
                popup.redraw = true;
            }
        }
    }
}

impl Dispatch<KdePrimaryOutputV1, ()> for Popup {
    fn event(
        popup: &mut Self,
        _: &KdePrimaryOutputV1,
        event: kde_primary_output_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let kde_primary_output_v1::Event::PrimaryOutput { output_name } = event {
            popup.primary_name = Some(output_name);
        }
    }
}

delegate_noop!(Popup: WpFractionalScaleManagerV1);
delegate_noop!(Popup: WpViewporter);
delegate_noop!(Popup: WpViewport);
delegate_compositor!(Popup);
delegate_output!(Popup);
delegate_shm!(Popup);
delegate_layer!(Popup);
delegate_seat!(Popup);
delegate_pointer!(Popup);
delegate_registry!(Popup);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kde_animation_speed_zero_means_no_motion() {
        assert!(kde_animations_off("[KDE]\nAnimationDurationFactor=0\n"));
        assert!(kde_animations_off(
            "[General]\nx=1\n[KDE]\nAnimationDurationFactor=0.0"
        ));
        assert!(!kde_animations_off("[KDE]\nAnimationDurationFactor=0.5\n"));
        assert!(!kde_animations_off("[Other]\nAnimationDurationFactor=0\n"));
        assert!(!kde_animations_off(""));
    }

    #[test]
    fn frames_fade_as_premultiplied_bgra() {
        let mut target = [0u8; 8];
        copy_bgra(&[10, 20, 30, 255, 200, 100, 50, 200], &mut target, 1.0);
        assert_eq!(target, [30, 20, 10, 255, 50, 100, 200, 200]);
        copy_bgra(&[10, 20, 30, 255, 200, 100, 50, 200], &mut target, 0.5);
        assert_eq!(target, [15, 10, 5, 128, 25, 50, 100, 100]);
    }

    #[test]
    fn an_offset_moves_the_window_beyond_the_slide_room() {
        let setup = OsdConfig {
            offset_pct: 10.0,
            ..OsdConfig::default()
        };
        let (layout, inset) = Popup::placement(setup.normalized(), false, Some((2560, 1440)));
        // 10 % of 2560: 256 DIPs; the window keeps 46 for the slide and moves the rest.
        assert_eq!(layout.pad, 20.0 + 46.0);
        assert_eq!(inset, 256 + 20 - 66);
        let (_, inset) = Popup::placement(setup.normalized(), false, None);
        assert_eq!(inset, 0, "an unknown output: no offset until it is known");
    }

    /// Private visual QA on a live Wayland session: every look on the primary output, each
    /// captured with Spectacle into `.artifacts/screenshots/live/<name>.png`. Never touches the
    /// daemon's control socket or audio. `cargo test -p macafond live_layer_shell -- --ignored`
    #[test]
    #[ignore = "shows popups on the live desktop"]
    fn live_layer_shell_demo() {
        use super::super::StatusIcon;
        use super::super::dictation::{State, View};
        use macafon_config::{OsdLook, OsdStyle};
        use macafon_ipc::DictationPopupLook::{Analog, Flow};
        assert!(supported(), "no layer-shell compositor");
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.artifacts/screenshots/live");
        std::fs::create_dir_all(&directory).expect("screenshot directory");
        let setup = |look, style, edge, offset_pct| OsdConfig {
            look,
            primary_monitor: true,
            style,
            edge,
            offset_pct,
        };
        let volume = |name: &str, color: &str, icon: &str, volume_pct, muted| Osd::Volume {
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
        let dictation = |look, state, hands_free| {
            Osd::Dictation(View {
                look,
                state,
                rms_dbfs: -16.0,
                bands: std::array::from_fn(|n| 0.3 + (n as f32 * 0.8).sin().abs() * 0.6),
                elapsed_s: 7,
                engine: "WHISPER".into(),
                words: 14,
                language: "EN".into(),
                hands_free,
            })
        };
        let dictation_setup = setup(OsdLook::Classic, OsdStyle::Horizontal, OsdEdge::Bottom, 0.0);
        use OsdEdge::{Bottom, Left, Right, Top};
        use OsdLook::{Classic, Sketch};
        use OsdStyle::{Horizontal, Vertical};
        let cases = [
            (
                "classic-vertical-music",
                volume("Music", "pink", "music", 67.0, false),
                setup(Classic, Vertical, Right, 0.0),
            ),
            (
                "classic-vertical-voice-muted",
                volume("Voice", "violet", "message", 40.0, true),
                setup(Classic, Vertical, Right, 0.0),
            ),
            (
                "sketch-vertical-left-game",
                volume("Game", "lime", "gamepad", 80.0, false),
                setup(Sketch, Vertical, Left, 0.0),
            ),
            (
                "classic-horizontal-master",
                volume("Master", "teal", "speaker", 52.0, false),
                setup(Classic, Horizontal, Bottom, 0.0),
            ),
            (
                "sketch-horizontal-top-music",
                volume("Music", "pink", "music", 67.0, false),
                setup(Sketch, Horizontal, Top, 0.0),
            ),
            (
                "classic-offset-music",
                volume("Music", "pink", "music", 30.0, false),
                setup(Classic, Vertical, Right, 10.0),
            ),
            (
                "classic-mic-muted",
                status(StatusIcon::MicOff, "Microphone", "Muted", true),
                setup(Classic, Vertical, Right, 0.0),
            ),
            (
                "classic-bypass",
                status(StatusIcon::Processing, "Processing", "Bypassed", true),
                setup(Classic, Horizontal, Bottom, 0.0),
            ),
            (
                "sketch-preset",
                status(StatusIcon::Preset, "Microphone preset", "Natural", false),
                setup(Sketch, Vertical, Right, 0.0),
            ),
            (
                "sketch-display",
                status(StatusIcon::Monitor, "Display", "Work", false),
                setup(Sketch, Horizontal, Bottom, 0.0),
            ),
            (
                "flow-listening-hands-free",
                dictation(Flow, State::Listening, true),
                dictation_setup,
            ),
            (
                "flow-transcribing",
                dictation(Flow, State::Transcribing, false),
                dictation_setup,
            ),
            (
                "flow-copied",
                dictation(Flow, State::Copied, false),
                dictation_setup,
            ),
            (
                "analog-listening",
                dictation(Analog, State::Listening, false),
                dictation_setup,
            ),
        ];
        for (name, view, setup) in cases {
            println!("POPUP {name}");
            let path = directory.join(format!("{name}.png"));
            let started = Instant::now();
            let mut capture: Option<std::process::Child> = None;
            loop {
                // Kept up like a held key or a live dictation.
                assert!(show(view.clone(), setup).is_none());
                std::thread::sleep(Duration::from_millis(150));
                if capture.is_none() && started.elapsed() > Duration::from_millis(900) {
                    capture = Some(
                        std::process::Command::new("spectacle")
                            .args(["-b", "-n", "-f", "-o"])
                            .arg(&path)
                            .spawn()
                            .expect("spectacle"),
                    );
                }
                if let Some(child) = capture.as_mut()
                    && child.try_wait().expect("spectacle").is_some()
                {
                    break;
                }
            }
            // Let it leave, so each capture shows one popup.
            std::thread::sleep(Duration::from_millis(2300));
        }
        // A cancelled dictation closes at once.
        assert!(show(dictation(Flow, State::Listening, false), dictation_setup).is_none());
        std::thread::sleep(Duration::from_millis(300));
        assert!(show(dictation(Flow, State::Cancelled, false), dictation_setup).is_none());
        std::thread::sleep(Duration::from_millis(300));
    }
}
