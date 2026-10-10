//! Tray icon (PRD §8.1): a D-Bus StatusNotifierItem served by `ksni` on its own thread.
//! Menu actions become ordinary control requests; the control loop pushes state back.

use crate::host::ControlMessage;
use ksni::blocking::{Handle, TrayMethods};
use miceq_backend_pipewire::ControlSender;
use miceq_ipc::{PresetInfo, Request};
use std::sync::mpsc;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrayState {
    pub muted: bool,
    pub bypass: bool,
    /// The selected microphone is linked; otherwise the icon shows the error dot.
    pub connected: bool,
    pub preset: Option<String>,
    pub preset_modified: bool,
    pub presets: Vec<PresetInfo>,
}

struct MicTray {
    state: TrayState,
    control: ControlSender<ControlMessage>,
}

impl MicTray {
    fn send(&self, request: Request) {
        // Nobody waits for the reply; the new state comes back through `Tray::update`.
        let (reply, _) = mpsc::channel();
        let _ = self.control.send(ControlMessage { request, reply });
    }
}

/// Look for the UI next to the daemon first, then on PATH (BG-004).
fn open_ui() {
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("miceq")))
        .filter(|path| path.is_file());
    let program = sibling.unwrap_or_else(|| "miceq".into());
    match std::process::Command::new(&program).spawn() {
        // Reap the child in the background so it does not linger as a zombie.
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(error) => {
            tracing::warn!(%error, program = %program.display(), "Cannot open micEQ")
        }
    }
}

impl ksni::Tray for MicTray {
    fn id(&self) -> String {
        "miceq".into()
    }

    fn title(&self) -> String {
        "micEQ".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::Hardware
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        icon::render(&self.state)
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let state = &self.state;
        let preset = state
            .preset
            .as_ref()
            .and_then(|id| state.presets.iter().find(|preset| &preset.id == id))
            .map_or("Custom".to_owned(), |preset| preset.name.clone());
        let mut lines = vec![if state.preset_modified {
            format!("Preset: {preset} (modified)")
        } else {
            format!("Preset: {preset}")
        }];
        if !state.connected {
            lines.push("Microphone disconnected".into());
        }
        if state.muted {
            lines.push("Microphone muted".into());
        }
        if state.bypass {
            lines.push("Processing bypassed".into());
        }
        ksni::ToolTip {
            title: "micEQ".into(),
            description: lines.join("\n"),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        open_ui();
    }

    /// BG-004: middle click toggles mute.
    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.send(Request::MicToggleMute);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        let presets = &self.state.presets;
        let selected = self
            .state
            .preset
            .as_ref()
            .and_then(|id| presets.iter().position(|preset| &preset.id == id))
            .unwrap_or(presets.len());
        let ids: Vec<String> = presets.iter().map(|preset| preset.id.clone()).collect();
        vec![
            StandardItem {
                label: "Open micEQ".into(),
                activate: Box::new(|_: &mut Self| open_ui()),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            CheckmarkItem {
                label: "Mute microphone".into(),
                checked: self.state.muted,
                activate: Box::new(|tray: &mut Self| {
                    tray.send(Request::MicSetMuted {
                        muted: !tray.state.muted,
                    })
                }),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Bypass processing".into(),
                checked: self.state.bypass,
                activate: Box::new(|tray: &mut Self| {
                    tray.send(Request::MicSetBypass {
                        bypass: !tray.state.bypass,
                    })
                }),
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Preset".into(),
                submenu: vec![
                    RadioGroup {
                        selected,
                        select: Box::new(move |tray: &mut Self, index| {
                            if let Some(id) = ids.get(index) {
                                tray.send(Request::MicSetPreset { preset: id.clone() });
                            }
                        }),
                        options: presets
                            .iter()
                            .map(|preset| RadioItem {
                                label: preset.name.clone(),
                                ..Default::default()
                            })
                            .collect(),
                    }
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit micEQ".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|tray: &mut Self| tray.send(Request::DaemonQuit)),
                ..Default::default()
            }
            .into(),
        ]
    }

    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        // No tray host yet (the panel starts later, or GNOME without the AppIndicator
        // extension): keep serving; the icon appears once a host shows up.
        tracing::info!(?reason, "No system tray available; waiting for one");
        true
    }
}

pub struct Tray {
    handle: Handle<MicTray>,
    shown: TrayState,
}

impl Tray {
    /// Start the tray service; `None` without a D-Bus session.
    pub fn spawn(state: TrayState, control: ControlSender<ControlMessage>) -> Option<Self> {
        let tray = MicTray {
            state: state.clone(),
            control,
        };
        match tray.assume_sni_available(true).spawn() {
            Ok(handle) => Some(Self {
                handle,
                shown: state,
            }),
            Err(error) => {
                tracing::warn!(%error, "Tray icon unavailable");
                None
            }
        }
    }

    pub fn update(&mut self, state: TrayState) {
        if state != self.shown {
            self.shown = state.clone();
            self.handle.update(|tray| tray.state = state);
        }
    }
}

/// The tray glyph, drawn at runtime so no image assets are needed: a microphone in
/// `--accent`, outlined when bypassed, slashed in `--danger` when muted, with a `--danger`
/// dot when the microphone is disconnected. Shape, not only colour, carries each state.
mod icon {
    use super::TrayState;

    const ACCENT: [u8; 3] = [0x2d, 0x81, 0xff];
    const DANGER: [u8; 3] = [0xff, 0x62, 0x59];
    const SIZES: [usize; 4] = [16, 22, 32, 48];

    type Point = (f32, f32);

    fn segment_distance(p: Point, a: Point, b: Point) -> f32 {
        let (pa, ba) = ((p.0 - a.0, p.1 - a.1), (b.0 - a.0, b.1 - a.1));
        let t = ((pa.0 * ba.0 + pa.1 * ba.1) / (ba.0 * ba.0 + ba.1 * ba.1)).clamp(0.0, 1.0);
        ((pa.0 - ba.0 * t).powi(2) + (pa.1 - ba.1 * t).powi(2)).sqrt()
    }

    /// Signed distances (negative inside) in unit coordinates.
    fn capsule(p: Point) -> f32 {
        segment_distance(p, (0.5, 0.27), (0.5, 0.43)) - 0.15
    }

    fn holder(p: Point) -> f32 {
        let (dx, dy) = (p.0 - 0.5, p.1 - 0.42);
        let ring = ((dx * dx + dy * dy).sqrt() - 0.27).abs() - 0.035;
        if dy < 0.0 {
            // Upper half of the ring is open: distance to its end points instead.
            ring.max(-dy)
        } else {
            ring
        }
        .min(segment_distance(p, (0.5, 0.69), (0.5, 0.86)) - 0.035)
        .min(segment_distance(p, (0.35, 0.87), (0.65, 0.87)) - 0.035)
    }

    fn slash(p: Point) -> f32 {
        segment_distance(p, (0.2, 0.1), (0.8, 0.9)) - 0.05
    }

    fn dot(p: Point) -> f32 {
        ((p.0 - 0.8).powi(2) + (p.1 - 0.8).powi(2)).sqrt() - 0.15
    }

    pub fn render(state: &TrayState) -> Vec<ksni::Icon> {
        SIZES.iter().map(|&size| draw(state, size)).collect()
    }

    fn draw(state: &TrayState, size: usize) -> ksni::Icon {
        let mut data = Vec::with_capacity(size * size * 4);
        let pixel = 1.0 / size as f32;
        let coverage = |distance: f32| (0.5 - distance / pixel).clamp(0.0, 1.0);
        for y in 0..size {
            for x in 0..size {
                let p = ((x as f32 + 0.5) * pixel, (y as f32 + 0.5) * pixel);
                let body = if state.bypass {
                    capsule(p).abs() - 0.035
                } else {
                    capsule(p)
                };
                let mut layers = vec![(coverage(body.min(holder(p))), ACCENT)];
                if state.muted {
                    layers.push((coverage(slash(p)), DANGER));
                }
                if !state.connected {
                    layers.push((coverage(dot(p)), DANGER));
                }
                // "Over" compositing in premultiplied space, then back to straight alpha.
                let (mut alpha, mut premultiplied) = (0.0f32, [0.0f32; 3]);
                for (cover, color) in layers {
                    for (channel, value) in premultiplied.iter_mut().zip(color) {
                        *channel = f32::from(value) * cover + *channel * (1.0 - cover);
                    }
                    alpha = cover + alpha * (1.0 - cover);
                }
                let un = |channel: f32| {
                    if alpha > 0.0 {
                        (channel / alpha).round().min(255.0) as u8
                    } else {
                        0
                    }
                };
                data.extend([
                    (alpha * 255.0).round() as u8,
                    un(premultiplied[0]),
                    un(premultiplied[1]),
                    un(premultiplied[2]),
                ]);
            }
        }
        ksni::Icon {
            width: size as i32,
            height: size as i32,
            data,
        }
    }
}
