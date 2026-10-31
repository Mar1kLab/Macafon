//! SVG is an internal drawing command buffer, rasterized locally with resvg/tiny-skia.
use super::super::{
    Osd, StatusIcon,
    dictation::State,
    icons::ICONS,
    motion::{BAR_H, BAR_W, STATUS_H, VOLUME_H, VOLUME_W, channel_colors},
};
use macafon_config::{OsdConfig, OsdLook, OsdStyle};
// Design tokens from docs/design/design-system.md, as in the Direct2D painter.
const BG: &str = "#101014";
const SUNKEN: &str = "#060607";
const INK: &str = "#f2f1ee";
const DANGER: &str = "#ff6259";
const AMBER: &str = "#ffc14d";
const ACCENT: &str = "#74aaff";
const FLOW: &str = "#0b0b0c";
pub(super) fn size(view: &Osd, setup: OsdConfig) -> (u32, u32) {
    let (w, h) = match view {
        Osd::Volume { .. } if setup.style == OsdStyle::Vertical => {
            (VOLUME_W as u32, VOLUME_H as u32)
        }
        Osd::Volume { .. } => (BAR_W as u32, BAR_H as u32),
        Osd::Status { .. } => (280, STATUS_H as u32),
        Osd::Dictation(v) => {
            if v.look == macafon_ipc::DictationPopupLook::Analog {
                (180, 72)
            } else {
                (180, 40)
            }
        }
    };
    (w + 40, h + 40)
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn text(x: f32, y: f32, size: u32, color: &str, value: &str) -> String {
    format!(
        r#"<text x="{x}" y="{y}" font-size="{size}" fill="{color}" font-family="Geist Mono">{}</text>"#,
        escape(value)
    )
}
fn icon(id: &str, x: f32, y: f32, color: &str) -> String {
    let nodes = ICONS
        .iter()
        .find(|(key, _)| *key == id)
        .map_or("", |(_, svg)| *svg);
    format!(
        r#"<g transform="translate({x} {y})" stroke="{color}" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" fill="none">{nodes}</g>"#
    )
}
pub(super) fn svg(
    view: &Osd,
    setup: OsdConfig,
    enter: f32,
    level: f32,
    vu: f32,
    time: f32,
) -> String {
    let (width, height) = size(view, setup);
    let (w, h) = ((width - 40) as f32, (height - 40) as f32);
    let sketch = setup.look == OsdLook::Sketch && !matches!(view, Osd::Dictation(_));
    let radius = if matches!(view, Osd::Dictation(v) if v.look == macafon_ipc::DictationPopupLook::Flow)
    {
        h / 2.0
    } else if sketch {
        0.0
    } else if matches!(view, Osd::Volume { .. }) && setup.style == OsdStyle::Vertical {
        34.0
    } else {
        8.0
    };
    let alpha = enter.clamp(0.0, 1.0);
    let scale = 0.9 + 0.1 * enter;
    let mut s = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}"><g opacity="{alpha}" transform="translate({} {}) scale({scale})"><rect width="{w}" height="{h}" rx="{radius}" fill="{BG}" stroke="{INK}" stroke-opacity="0.15"/>"#,
        20.0 + (1.0 - scale) * w / 2.0,
        20.0 + (1.0 - scale) * h / 2.0
    );
    if sketch {
        for x in [0.0, w] {
            for y in [0.0, h] {
                s += &format!(
                    r#"<path d="M{} {y}h11M{x} {}v11" stroke="{INK}" stroke-opacity="0.4"/>"#,
                    x - 5.5,
                    y - 5.5
                );
            }
        }
    }
    match view {
        Osd::Volume {
            name,
            color,
            icon: id,
            muted,
            ..
        } => {
            let (fill, ink) = channel_colors(color);
            let fill = if *muted {
                DANGER.into()
            } else {
                format!("#{fill:06x}")
            };
            let ink = format!("#{ink:06x}");
            let horizontal = setup.style == OsdStyle::Horizontal;
            if !sketch && !horizontal {
                let value = if *muted {
                    "Muted".into()
                } else {
                    format!("{:.0}%", level.clamp(0.0, 1.0) * 100.0)
                };
                s += &text(12.0, 30.0, 13, if *muted { DANGER } else { INK }, &value);
                let track_h = h - 90.0;
                let length = 48.0 + (track_h - 48.0) * level.clamp(0.0, 1.0);
                let top = h - 44.0 - length;
                s += &format!(
                    r#"<defs><linearGradient id="level" x2="0" y2="1"><stop stop-color="{ink}"/><stop offset="1" stop-color="{fill}"/></linearGradient></defs><rect x="10" y="46" width="48" height="{track_h}" rx="24" fill="{INK}" fill-opacity="0.1"/><rect x="10" y="{top}" width="48" height="{length}" rx="24" fill="url(#level)"/>"#
                );
                s += &icon(if *muted { "volume-x" } else { id }, 22.0, h - 80.0, INK);
                s += &text(
                    8.0,
                    h - 22.0,
                    12,
                    &ink,
                    &name.chars().take(8).collect::<String>(),
                );
            } else {
                let (x, y, tw, th) = if horizontal {
                    (54.0, 34.0, w - 118.0, 14.0)
                } else {
                    (14.0, 46.0, 40.0, h - 92.0)
                };
                let level = level.clamp(0.0, 1.0);
                s += &format!(
                    r#"<rect x="{x}" y="{y}" width="{tw}" height="{th}" rx="{}" fill="{SUNKEN}"/>"#,
                    if sketch { 0 } else { 7 }
                );
                let (fx, fy, fw, fh) = if horizontal {
                    (x, y, tw * level, th)
                } else {
                    (x, y + th * (1.0 - level), tw, th * level)
                };
                s += &format!(
                    r#"<rect x="{fx}" y="{fy}" width="{fw}" height="{fh}" fill="{fill}" opacity="{}"/>"#,
                    if *muted { 0.35 } else { 1.0 }
                );
                if sketch {
                    for n in 0..=10 {
                        let t = n as f32 / 10.0;
                        let len = if n % 5 == 0 { 7.0 } else { 4.0 };
                        let (tx, ty, path) = if horizontal {
                            (x + tw * t, y + th, format!("v-{len}"))
                        } else {
                            (x, y + th * (1.0 - t), format!("h{len}"))
                        };
                        s += &format!(
                            r#"<path d="M{tx} {ty}{path}" stroke="{INK}" stroke-opacity="0.4"/>"#
                        );
                    }
                }
                s += &icon(
                    if *muted { "volume-x" } else { id },
                    if horizontal { 16.0 } else { 22.0 },
                    if horizontal { 18.0 } else { 12.0 },
                    &ink,
                );
                let label = if *muted {
                    "MUTED".into()
                } else {
                    format!("{:.0}%", level * 100.0)
                };
                s += &text(
                    if horizontal { w - 58.0 } else { 12.0 },
                    if horizontal { 38.0 } else { h - 26.0 },
                    12,
                    INK,
                    &label,
                );
                // The card clips long names; escape all user-provided labels before XML parsing.
                s += &text(
                    if horizontal { 54.0 } else { 8.0 },
                    if horizontal { 23.0 } else { h - 10.0 },
                    10,
                    INK,
                    &name
                        .chars()
                        .take(if horizontal { 28 } else { 8 })
                        .collect::<String>(),
                );
            }
        }
        Osd::Status {
            icon: id,
            title,
            value,
            alert,
        } => {
            let color = if *alert { DANGER } else { ACCENT };
            s += &icon(
                match id {
                    StatusIcon::Mic => "mic",
                    StatusIcon::MicOff => "mic-off",
                    StatusIcon::Processing => "processing",
                    StatusIcon::Preset => "preset",
                    StatusIcon::Monitor => "monitor",
                },
                16.0,
                20.0,
                color,
            );
            s += &text(
                54.0,
                25.0,
                12,
                INK,
                &title.chars().take(26).collect::<String>(),
            );
            s += &text(
                54.0,
                47.0,
                16,
                color,
                &value.chars().take(22).collect::<String>(),
            );
        }
        Osd::Dictation(v) => {
            let color = if v.state == State::Error {
                DANGER
            } else if v.elapsed_s >= 270 {
                AMBER
            } else {
                INK
            };
            if v.look == macafon_ipc::DictationPopupLook::Flow {
                s += &format!(r#"<rect width="{w}" height="{h}" rx="20" fill="{FLOW}"/>"#);
                match v.state {
                    State::Listening => {
                        for n in 0..15 {
                            let band = v.bands[n.min(14 - n)];
                            let bh = 3.0
                                + band.clamp(0.0, 1.0)
                                    * 18.0
                                    * (1.0 - ((n as f32 - 7.0) / 8.0).abs() * 0.25);
                            s += &format!(
                                r#"<rect x="{}" y="{}" width="3" height="{bh}" rx="1.5" fill="{color}"/>"#,
                                if v.hands_free { 14 + n * 6 } else { 45 + n * 6 },
                                (h - bh) / 2.0
                            );
                        }
                    }
                    State::Transcribing => {
                        for (n, x) in [78, 90, 102].into_iter().enumerate() {
                            let opacity = 0.3
                                + 0.7
                                    * ((time * std::f32::consts::TAU / 0.9 - n as f32 * 0.8).sin()
                                        * 0.5
                                        + 0.5);
                            s += &format!(
                                r#"<circle cx="{x}" cy="20" r="3" fill="{color}" opacity="{opacity}"/>"#
                            );
                        }
                    }
                    _ => s += &text(32.0, 25.0, 12, color, state_label(v.state)),
                }
            } else {
                s += &text(12.0, 16.0, 10, color, "VU · DICTATION");
                s += &format!(
                    r#"<path d="M24 49 Q90 4 156 49" fill="none" stroke="{INK}" stroke-opacity="0.4"/>"#
                );
                let travel = ((vu - 0.1) / (10.0f32.powf(3.0 / 20.0) - 0.1)).clamp(-0.02, 1.08);
                let angle = -1.0 + 2.0 * travel;
                s += &format!(
                    r#"<path d="M90 58 L{} {}" stroke="{AMBER}" stroke-width="2"/>"#,
                    90.0 + angle.sin() * 47.0,
                    58.0 - angle.cos() * 47.0
                );
                s += &text(12.0, 68.0, 10, color, state_label(v.state));
            }
            if v.hands_free && v.state == State::Listening {
                s += &format!(
                    r#"<rect x="{}" y="14" width="10" height="10" fill="{INK}"/><path d="M{} 14l10 10m0-10l-10 10" stroke="{INK}" stroke-width="2"/>"#,
                    w - 46.0,
                    w - 24.0
                );
            }
        }
    }
    s + "</g></svg>"
}
// Same English native-popup labels as the Windows painter (UI configuration remains i18n).
fn state_label(state: State) -> &'static str {
    match state {
        State::Listening => "LISTENING",
        State::Transcribing => "TRANSCRIBING",
        State::Done => "TYPED",
        State::Copied => "COPIED",
        State::NoSpeech => "No speech",
        State::Error => "Dictation error",
        State::Cancelled => "",
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn user_labels_are_xml_text() {
        let view = Osd::Status {
            icon: StatusIcon::Monitor,
            title: "<work & play>".into(),
            value: "\"wide\"".into(),
            alert: false,
        };
        let svg = svg(&view, OsdConfig::default(), 1.0, 0.5, 0.5, 0.0);
        assert!(svg.contains("&lt;work &amp; play&gt;"));
        assert!(resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).is_ok());
    }
}
