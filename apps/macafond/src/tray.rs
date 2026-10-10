//! Tray icon (PRD §8.1): a D-Bus StatusNotifierItem served by `ksni` on its own thread.
//! Menu actions become ordinary control requests; the control loop pushes state back.

use crate::host::ControlMessage;
use ksni::blocking::{Handle, TrayMethods};
use macafon_backend_pipewire::ControlSender;
use macafon_ipc::{PresetInfo, Request};
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
    bend: f32,
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
        .and_then(|exe| Some(exe.parent()?.join("macafon")))
        .filter(|path| path.is_file());
    let program = sibling.unwrap_or_else(|| "macafon".into());
    match std::process::Command::new(&program).spawn() {
        // Reap the child in the background so it does not linger as a zombie.
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(error) => {
            tracing::warn!(%error, program = %program.display(), "Cannot open Macafon")
        }
    }
}

impl ksni::Tray for MicTray {
    fn id(&self) -> String {
        "macafon".into()
    }

    fn title(&self) -> String {
        "Macafon".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::Hardware
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        icon::render(&self.state, self.bend)
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
            title: "Macafon".into(),
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
                label: "Open Macafon".into(),
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
                label: "Quit Macafon".into(),
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
    updates: mpsc::Sender<TrayState>,
    shown: TrayState,
}

impl Tray {
    /// Start the tray service; `None` without a D-Bus session.
    pub fn spawn(state: TrayState, control: ControlSender<ControlMessage>) -> Option<Self> {
        let tray = MicTray {
            bend: icon::target(&state),
            state: state.clone(),
            control,
        };
        match tray.assume_sni_available(true).spawn() {
            Ok(handle) => {
                let (updates, receiver) = mpsc::channel();
                let bend = icon::target(&state);
                // SNI updates and animation run off the control and audio threads.
                std::thread::spawn(move || animate(handle, receiver, bend));
                Some(Self {
                    updates,
                    shown: state,
                })
            }
            Err(error) => {
                tracing::warn!(%error, "Tray icon unavailable");
                None
            }
        }
    }

    pub fn update(&mut self, state: TrayState) {
        if state != self.shown {
            self.shown = state.clone();
            let _ = self.updates.send(state);
        }
    }
}

/// Sleep while idle; animate only a state transition and retarget rapid toggles.
fn animate(handle: Handle<MicTray>, updates: mpsc::Receiver<TrayState>, mut bend: f32) {
    use std::time::{Duration, Instant};
    const FRAME: Duration = Duration::from_millis(25);
    const DURATION: f32 = 0.3;
    'service: while let Ok(mut state) = updates.recv() {
        while let Ok(next) = updates.try_recv() {
            state = next;
        }
        let mut from = bend;
        let mut target = icon::target(&state);
        let mut started = Instant::now();
        loop {
            if handle
                .update(|tray| {
                    tray.state = state.clone();
                    tray.bend = bend;
                })
                .is_none()
            {
                break 'service;
            }
            if bend == target {
                break;
            }
            match updates.recv_timeout(FRAME) {
                Ok(next) => {
                    state = next;
                    while let Ok(next) = updates.try_recv() {
                        state = next;
                    }
                    from = bend;
                    target = icon::target(&state);
                    started = Instant::now();
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let progress = (started.elapsed().as_secs_f32() / DURATION).min(1.0);
                    bend = if progress == 1.0 {
                        target
                    } else {
                        let eased = 0.5 - 0.5 * (std::f32::consts::PI * progress).cos();
                        from + (target - from) * eased
                    };
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break 'service,
            }
        }
    }
    handle.shutdown().wait();
}

/// Approved rounded microphone; generated masks are shared with the UI's rubber warp.
mod icon {
    use super::TrayState;

    const WHITE: [u8; 3] = [255; 3];
    const DANGER: [u8; 3] = [0xff, 0x62, 0x59];
    const SIZES: [usize; 4] = [16, 22, 32, 48];
    const FRAME_BYTES: usize = 16 * 16 + 22 * 22 + 32 * 32 + 48 * 48;
    const STEPS: usize = 12;
    const MASKS: &[u8; FRAME_BYTES * (STEPS + 1)] = include_bytes!("../assets/logo-alpha.bin");

    pub fn target(state: &TrayState) -> f32 {
        if state.muted || state.bypass || !state.connected {
            1.0
        } else {
            0.0
        }
    }

    fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
        let (pa, ba) = ((p.0 - a.0, p.1 - a.1), (b.0 - a.0, b.1 - a.1));
        let t = ((pa.0 * ba.0 + pa.1 * ba.1) / (ba.0 * ba.0 + ba.1 * ba.1)).clamp(0.0, 1.0);
        ((pa.0 - ba.0 * t).powi(2) + (pa.1 - ba.1 * t).powi(2)).sqrt()
    }

    pub fn render(state: &TrayState, bend: f32) -> Vec<ksni::Icon> {
        let bend = bend.clamp(0.0, 1.0);
        let frame = (bend * STEPS as f32).round() as usize;
        let color = std::array::from_fn::<_, 3, _>(|i| {
            (f32::from(WHITE[i]) * (1.0 - bend) + f32::from(DANGER[i]) * bend).round() as u8
        });
        let mut offset = frame * FRAME_BYTES;
        SIZES
            .iter()
            .map(|&size| {
                let alpha = &MASKS[offset..offset + size * size];
                offset += size * size;
                let mut data = Vec::with_capacity(size * size * 4);
                for (index, &alpha) in alpha.iter().enumerate() {
                    // Original vector coordinates, including its -8 viewBox offset.
                    let x = (index % size) as f32;
                    let y = (index / size) as f32;
                    let p = (
                        (x + 0.5) * 128.0 / size as f32 - 8.0,
                        (y + 0.5) * 128.0 / size as f32,
                    );
                    let coverage =
                        |distance: f32| (0.5 - distance * size as f32 / 128.0).clamp(0.0, 1.0);
                    let pause =
                        if state.bypass {
                            coverage(
                                segment_distance(p, (96.0, 88.0), (96.0, 105.0))
                                    .min(segment_distance(p, (106.0, 88.0), (106.0, 105.0)))
                                    - 2.5,
                            )
                        } else {
                            0.0
                        };
                    let dot = if !state.connected {
                        coverage(((p.0 - 105.0).powi(2) + (p.1 - 22.0).powi(2)).sqrt() - 8.0)
                    } else {
                        0.0
                    };
                    let base = (f32::from(alpha) / 255.0).max(pause);
                    let total = dot + base * (1.0 - dot);
                    let rgb = std::array::from_fn::<_, 3, _>(|i| {
                        if total == 0.0 {
                            0
                        } else {
                            ((f32::from(DANGER[i]) * dot
                                + f32::from(color[i]) * base * (1.0 - dot))
                                / total)
                                .round() as u8
                        }
                    });
                    data.extend([(total * 255.0).round() as u8, rgb[0], rgb[1], rgb[2]]);
                }
                ksni::Icon {
                    width: size as i32,
                    height: size as i32,
                    data,
                }
            })
            .collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn states_and_transition_frames_remain_distinct() {
            let normal = TrayState {
                connected: true,
                ..Default::default()
            };
            let muted = TrayState {
                muted: true,
                ..normal.clone()
            };
            let bypass = TrayState {
                bypass: true,
                ..normal.clone()
            };
            let error = TrayState {
                connected: false,
                ..normal.clone()
            };
            let normal_icons = render(&normal, target(&normal));
            let muted_icons = render(&muted, target(&muted));
            let bypass_icons = render(&bypass, target(&bypass));
            let error_icons = render(&error, target(&error));
            let halfway = render(&muted, 0.5);
            for (i, &size) in SIZES.iter().enumerate() {
                assert_eq!(normal_icons[i].data.len(), size * size * 4);
                assert!(
                    normal_icons[i]
                        .data
                        .chunks_exact(4)
                        .any(|pixel| pixel[0] > 0)
                );
                assert_ne!(normal_icons[i].data, muted_icons[i].data);
                assert_ne!(muted_icons[i].data, bypass_icons[i].data);
                assert_ne!(muted_icons[i].data, error_icons[i].data);
                assert_ne!(normal_icons[i].data, halfway[i].data);
                assert_ne!(muted_icons[i].data, halfway[i].data);
                assert!(
                    normal_icons[i]
                        .data
                        .chunks_exact(4)
                        .filter(|p| p[0] > 0)
                        .all(|p| p[1..] == WHITE)
                );
            }
        }
    }
}
