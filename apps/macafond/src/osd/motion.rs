//! What both native popups share (HK-006, §9.9.3): the layout in DIPs, the springs and the
//! state machine that turns each change into motion. The platforms only place a window and
//! paint a `Scene`: Direct2D on Windows, tiny-skia into a layer-shell surface on Linux.
use super::dictation::{State as DictationState, View as DictationView, Vu};
use super::{Osd, StatusIcon};
use macafon_config::{OsdConfig, OsdEdge, OsdStyle};
use std::time::{Duration, Instant};

/// How long the popup stays after the last change.
pub(super) const HOLD: Duration = Duration::from_millis(1600);

// Layout in DIPs. The panel keeps `MARGIN` from its edge of the work area plus the offset from
// Settings; the window around it leaves room for the shadow, the overscroll and the slide.
pub(super) const MARGIN: f32 = 20.0;
/// The dictation popup sits higher: 48 DIPs above the work area's edge (§9.9.3).
pub(super) const DICTATION_MARGIN: f32 = 48.0;
pub(super) const SHADOW: f32 = 18.0;
pub(super) const SLIDE: f32 = 28.0;
/// How far an overscrolled slider stretches.
pub(super) const STRETCH: f32 = 12.0;
// Vertical style (Android-like): a pill-shaped slider.
pub(super) const VOLUME_W: f32 = 68.0;
pub(super) const VOLUME_H: f32 = 268.0;
// Horizontal style (Windows 11-like): a bar with the flyouts' small corners.
pub(super) const BAR_W: f32 = 340.0;
pub(super) const BAR_H: f32 = 60.0;
pub(super) const BAR_RADIUS: f32 = 8.0;
// The other actions, in both styles.
pub(super) const STATUS_H: f32 = 64.0;
pub(super) const STATUS_MIN_W: f32 = 196.0;
pub(super) const STATUS_MAX_W: f32 = 340.0;

// The Analog dictation popup: a small, quiet VU meter (§9.9.3).
pub(super) const ANALOG_W: f32 = 128.0;
pub(super) const ANALOG_H: f32 = 60.0;
/// Bars of the Flow pill: 15 bars of 3 px every 6 px.
pub(super) const FLOW_BARS_W: f32 = 87.0;

/// Needle travel 0…1 for a linear VU amplitude (1 = 0 VU): linear in voltage from −20 to
/// +3 VU, so 0 VU sits about two thirds along, as on real meters.
pub(super) fn vu_travel(amplitude: f32) -> f32 {
    let (low, high) = (0.1f32, 10f32.powf(3.0 / 20.0));
    ((amplitude - low) / (high - low)).clamp(-0.02, 1.08)
}

pub(super) fn dictation_size(view: &DictationView) -> (f32, f32) {
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

/// The panel of `view` in DIPs: what `Layout::place` positions.
pub(super) fn panel_size(view: &Osd, style: OsdStyle, status_width: f32) -> (f32, f32) {
    match view {
        Osd::Volume { .. } if style == OsdStyle::Horizontal => (BAR_W, BAR_H),
        Osd::Volume { .. } => (VOLUME_W, VOLUME_H),
        Osd::Status { .. } => (status_width, STATUS_H),
        Osd::Dictation(view) => dictation_size(view),
    }
}

/// `--app-<color>` fill and `--app-<color>-ink` text for each of `CHANNEL_COLORS`.
pub(super) fn channel_colors(color: &str) -> (u32, u32) {
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

/// The Lucide icon of a status card.
#[cfg_attr(windows, allow(dead_code))]
pub(super) fn status_icon(icon: StatusIcon) -> &'static str {
    match icon {
        StatusIcon::Mic => "mic",
        StatusIcon::MicOff => "mic-off",
        StatusIcon::Processing => "processing",
        StatusIcon::Preset => "preset",
        StatusIcon::Monitor => "monitor",
    }
}

/// A damped spring; `step` integrates in small substeps so a long frame cannot blow it up.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Spring {
    pub(super) value: f32,
    pub(super) velocity: f32,
    pub(super) target: f32,
}

impl Spring {
    pub(super) fn snap(&mut self, target: f32) {
        *self = Self {
            value: target,
            velocity: 0.0,
            target,
        };
    }

    pub(super) fn step(&mut self, dt: f32, frequency_hz: f32, damping: f32) {
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

    pub(super) fn settled(&self) -> bool {
        (self.value - self.target).abs() < 1e-3 && self.velocity.abs() < 1e-2
    }
}

/// Spring tunings: (frequency in Hz, damping ratio).
pub(super) const ENTER: (f32, f32) = (3.2, 0.62);
pub(super) const EXIT: (f32, f32) = (3.6, 1.0);
pub(super) const LEVEL: (f32, f32) = (7.0, 1.0);
pub(super) const FADE: (f32, f32) = (5.0, 1.0);
pub(super) const BOUNCE: (f32, f32) = (4.0, 0.35);
pub(super) const KICK: f32 = 18.0;

/// A rectangle in DIPs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Bounds {
    pub(super) left: f32,
    pub(super) top: f32,
    pub(super) right: f32,
    pub(super) bottom: f32,
}

pub(super) fn bounds(left: f32, top: f32, right: f32, bottom: f32) -> Bounds {
    Bounds {
        left,
        top,
        right,
        bottom,
    }
}

/// The distance from the work area's edge to the panel: the hotkey popup's or the dictation
/// popup's.
pub(super) fn edge_margin(dictation: bool) -> f32 {
    if dictation { DICTATION_MARGIN } else { MARGIN }
}

/// Where the panel sits in the window and which way it slides in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Layout {
    pub(super) setup: OsdConfig,
    /// From the window's edge to the panel: `MARGIN`, plus room for the slide once the offset
    /// moves the panel away from the screen edge (the window never leaves the work area).
    pub(super) pad: f32,
}

impl Layout {
    /// `margin` from `edge_margin`; `offset_dip` is the Settings offset on this monitor.
    pub(super) fn new(setup: OsdConfig, margin: f32, offset_dip: f32) -> Self {
        Self {
            setup,
            pad: margin + offset_dip.min(SLIDE + SHADOW),
        }
    }

    pub(super) fn window(&self) -> (f32, f32) {
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
    pub(super) fn place(
        &self,
        size: (f32, f32),
        panel: (f32, f32),
    ) -> (Bounds, (f32, f32), (f32, f32)) {
        let ((w, h), (pw, ph), pad) = (size, panel, self.pad);
        match self.setup.edge {
            OsdEdge::Right => (
                bounds(w - pad - pw, (h - ph) / 2.0, w - pad, (h + ph) / 2.0),
                (w - pad, h / 2.0),
                (1.0, 0.0),
            ),
            OsdEdge::Left => (
                bounds(pad, (h - ph) / 2.0, pad + pw, (h + ph) / 2.0),
                (pad, h / 2.0),
                (-1.0, 0.0),
            ),
            OsdEdge::Bottom => (
                bounds((w - pw) / 2.0, h - pad - ph, (w + pw) / 2.0, h - pad),
                (w / 2.0, h - pad),
                (0.0, 1.0),
            ),
            OsdEdge::Top => (
                bounds((w - pw) / 2.0, pad, (w + pw) / 2.0, pad + ph),
                (w / 2.0, pad),
                (0.0, -1.0),
            ),
        }
    }

    /// The panel's transform for `enter` as `(scale, dx, dy)`: it grows from its edge side and
    /// slides in from the screen edge; an overshoot of `enter` past 1 is the bounce.
    pub(super) fn transform(
        &self,
        size: (f32, f32),
        panel: (f32, f32),
        enter: f32,
    ) -> (f32, f32, f32) {
        let (_, (pivot_x, pivot_y), (toward_x, toward_y)) = self.place(size, panel);
        let scale = 0.86 + 0.14 * enter;
        (
            scale,
            (1.0 - scale) * pivot_x + (1.0 - enter) * SLIDE * toward_x,
            (1.0 - scale) * pivot_y + (1.0 - enter) * SLIDE * toward_y,
        )
    }
}

/// One frame: `enter` 0 → 1 (springs may overshoot), the rest in 0..=1 except `stretch`.
pub(super) struct Scene<'a> {
    pub(super) vu: f32,
    pub(super) vu_peak: f32,
    pub(super) shimmer: f32,
    pub(super) view: &'a Osd,
    pub(super) enter: f32,
    pub(super) level: f32,
    pub(super) mute: f32,
    pub(super) alert: f32,
    pub(super) stretch: f32,
    pub(super) status_width: f32,
}

/// What `Motion::advance` leaves the popup with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    /// Something moves: paint the next frame right away.
    Moving,
    /// Everything rests; wait for a change or the end of the hold.
    Resting,
    /// The exit finished: hide the window.
    Hidden,
}

/// The popup's state between frames: what it shows and every spring.
pub(super) struct Motion {
    pub(super) view: Option<Osd>,
    pub(super) visible: bool,
    pub(super) exiting: bool,
    pub(super) changed_at: Instant,
    pub(super) enter: Spring,
    pub(super) level: Spring,
    pub(super) mute: Spring,
    pub(super) alert: Spring,
    pub(super) stretch: Spring,
    pub(super) status_width: f32,
    pub(super) vu: Vu,
    /// The Analog needle's peak, held and let down slowly: a faint ghost needle.
    pub(super) vu_peak: f32,
    pub(super) shimmer: f32,
}

impl Motion {
    pub(super) fn new() -> Self {
        Self {
            view: None,
            visible: false,
            exiting: false,
            changed_at: Instant::now(),
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

    /// Whether `osd` continues what is shown: the same channel, action or dictation.
    pub(super) fn continues(&self, osd: &Osd) -> bool {
        match (&self.view, osd) {
            (Some(Osd::Dictation(_)), Osd::Dictation(_)) => true,
            (Some(Osd::Volume { name: a, .. }), Osd::Volume { name: b, .. }) => a == b,
            (Some(Osd::Status { title: a, .. }), Osd::Status { title: b, .. }) => a == b,
            _ => false,
        }
    }

    /// A change, once the platform has placed the window; `same` from `continues`.
    /// `status_width` measures a status card's title and value.
    pub(super) fn receive(
        &mut self,
        osd: Osd,
        same: bool,
        now: Instant,
        animations: bool,
        status_width: impl FnOnce(&str, &str) -> f32,
    ) {
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
                self.status_width = status_width(title, value);
            }
        }
        self.view = Some(osd);
        if !animations {
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

    /// Moves every spring by `dt`. `animations` is asked only when it matters.
    pub(super) fn advance(&mut self, now: Instant, dt: f32, animations: impl Fn() -> bool) -> Step {
        if !self.exiting && now - self.changed_at >= self.hold() {
            self.exiting = true;
            self.enter.target = 0.0;
            if !animations() {
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
            if animations() {
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
            return Step::Hidden;
        }
        if matches!(&self.view, Some(Osd::Dictation(v)) if matches!(v.state, DictationState::Listening | DictationState::Transcribing))
        {
            return Step::Moving;
        }
        if [self.enter, self.level, self.mute, self.alert, self.stretch]
            .iter()
            .all(Spring::settled)
        {
            Step::Resting
        } else {
            Step::Moving
        }
    }

    /// How long nothing needs a frame: the rest of the hold while shown, forever (`None`)
    /// while hidden or leaving.
    pub(super) fn wait(&self, now: Instant) -> Option<Duration> {
        if !self.visible || self.exiting {
            return None;
        }
        Some(self.hold().saturating_sub(now - self.changed_at))
    }

    pub(super) fn hold(&self) -> Duration {
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

    pub(super) fn hide(&mut self) {
        self.visible = false;
        self.exiting = false;
    }

    /// The frame to paint, if anything is shown.
    pub(super) fn scene(&self) -> Option<Scene<'_>> {
        Some(Scene {
            view: self.view.as_ref()?,
            enter: self.enter.value,
            level: self.level.value.clamp(0.0, 1.0),
            mute: self.mute.value.clamp(0.0, 1.0),
            alert: self.alert.value.clamp(0.0, 1.0),
            stretch: self.stretch.value,
            status_width: self.status_width,
            vu: self.vu.value,
            vu_peak: self.vu_peak,
            shimmer: self.shimmer,
        })
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

    #[test]
    fn the_panel_keeps_its_margin_from_every_edge() {
        let panel = (VOLUME_W, VOLUME_H);
        for edge in [OsdEdge::Right, OsdEdge::Left, OsdEdge::Top, OsdEdge::Bottom] {
            let style = if matches!(edge, OsdEdge::Left | OsdEdge::Right) {
                OsdStyle::Vertical
            } else {
                OsdStyle::Horizontal
            };
            let setup = OsdConfig {
                style,
                edge,
                ..OsdConfig::default()
            };
            // An offset of 100 DIPs: the slide room is capped, the rest moves the window.
            let layout = Layout::new(setup, MARGIN, 100.0);
            assert_eq!(layout.pad, MARGIN + SLIDE + SHADOW);
            let size = layout.window();
            let (b, _, toward) = layout.place(size, panel);
            let gap = match edge {
                OsdEdge::Right => size.0 - b.right,
                OsdEdge::Left => b.left,
                OsdEdge::Top => b.top,
                OsdEdge::Bottom => size.1 - b.bottom,
            };
            assert_eq!(gap, layout.pad, "{edge:?}");
            // Centered across the edge, sliding in from it.
            let (across, start, end) = match edge {
                OsdEdge::Left | OsdEdge::Right => (size.1, b.top, b.bottom),
                OsdEdge::Top | OsdEdge::Bottom => (size.0, b.left, b.right),
            };
            assert!((start + end - across).abs() < 1e-3, "{edge:?}");
            let (scale, dx, dy) = layout.transform(size, panel, 0.0);
            assert!((scale - 0.86).abs() < 1e-6);
            assert!(dx * toward.0 >= 0.0 && dy * toward.1 >= 0.0, "{edge:?}");
            assert_eq!(layout.transform(size, panel, 1.0), (1.0, 0.0, 0.0));
        }
    }

    fn volume(name: &str, volume_pct: f32) -> Osd {
        Osd::Volume {
            name: name.into(),
            color: "pink".into(),
            icon: "music".into(),
            volume_pct,
            muted: false,
        }
    }

    #[test]
    fn a_popup_holds_then_leaves_and_overscroll_kicks_at_the_limit() {
        let start = Instant::now();
        let mut motion = Motion::new();
        let same = motion.continues(&volume("Music", 100.0));
        motion.receive(volume("Music", 100.0), same, start, true, |_, _| 0.0);
        assert!(motion.visible && !same);
        assert_eq!(motion.level.value, 1.0, "a new channel jumps to its level");
        // Holding the key at 100 %: the slider stretches instead of moving.
        let same = motion.continues(&volume("Music", 100.0));
        motion.receive(volume("Music", 100.0), same, start, true, |_, _| 0.0);
        assert!(same && motion.stretch.velocity >= KICK);
        let mut now = start;
        let mut steps = 0;
        loop {
            now += Duration::from_millis(16);
            steps += 1;
            match motion.advance(now, 0.016, || true) {
                Step::Hidden => break,
                Step::Moving | Step::Resting => assert!(steps < 400, "never left"),
            }
        }
        assert!(!motion.visible);
        let shown = now - start;
        assert!(
            shown >= HOLD && shown < HOLD + Duration::from_millis(700),
            "{shown:?}"
        );
        assert_eq!(motion.wait(now), None, "hidden waits forever");
    }

    #[test]
    fn without_animations_every_spring_lands_at_once() {
        let now = Instant::now();
        let mut motion = Motion::new();
        motion.receive(volume("Main", 40.0), false, now, false, |_, _| 0.0);
        assert_eq!(motion.enter.value, 1.0);
        assert_eq!(motion.level.value, 0.4);
        assert_eq!(motion.advance(now, 0.0, || false), Step::Resting);
        assert_eq!(motion.advance(now + HOLD, 0.0, || false), Step::Hidden);
    }
}
