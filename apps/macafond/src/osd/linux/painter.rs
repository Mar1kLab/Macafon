//! The Linux popup's pictures: the Windows painter (`popup/painter.rs`) redrawn with tiny-skia
//! into a premultiplied RGBA pixmap, primitive for primitive and with the same geometry, so both
//! platforms show the same Classic and Sketch looks and dictation states.
use super::super::dictation::{State as DictationState, View as DictationView};
use super::super::icons::ICONS;
use super::super::motion::*;
use super::super::Osd;
use super::text::{Family, Fonts, Format, format};
use macafon_config::{OsdLook, OsdStyle};
use std::collections::HashMap;
use tiny_skia::{
    Color, FillRule, GradientStop, LinearGradient, Mask, Paint, Path, PathBuilder, Pixmap,
    PixmapMut, Point, Rect, SpreadMode, Stroke, Transform,
};

// Design tokens (docs/design/design-system.md), as in the Direct2D painter.
const BG_RAISED: u32 = 0x101014;
const SUNKEN: u32 = 0x060607;
const INK: u32 = 0xf2f1ee;
const WHITE: u32 = 0xffffff;
const BLACK: u32 = 0x000000;
const ACCENT: u32 = 0x2d81ff;
const DANGER: u32 = 0xff6259;
const AMBER: u32 = 0xffc14d;
const AMBER_DEEP: u32 = 0xf2a516;
const INVERT_FG: u32 = 0x0b0b0c;

// The Direct2D text formats, with Inter in place of Segoe UI Variable.
const MONO: Format = format(Family::Mono, 13.0, false);
const MONO_CENTER: Format = format(Family::Mono, 13.0, true);
const HUD: Format = format(Family::Mono, 11.0, false);
const HUD_CENTER: Format = format(Family::Mono, 11.0, true);
const PERCENT: Format = format(Family::SansStrong, 15.0, true);
const NAME: Format = format(Family::SansStrong, 12.0, true);
const LABEL: Format = format(Family::SansStrong, 12.0, false);
const TITLE: Format = format(Family::Sans, 12.0, false);
const VALUE: Format = format(Family::SansStrong, 16.0, false);
const DICTATION_HUD: Format = format(Family::Mono, 10.0, false);
const DICTATION_SERIF: Format = format(Family::Serif, 17.0, true);
const DICTATION_BODY: Format = format(Family::Sans, 12.0, false);
/// HUD labels: uppercase with DirectWrite's 1.65 DIP trailing spacing.
const HUD_SPACING: f32 = 1.65;

/// A color with straight alpha, mixed like `D2D1_COLOR_F`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rgba {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

fn rgba(hex: u32, a: f32) -> Rgba {
    let channel = |shift: u32| ((hex >> shift) & 0xff) as f32 / 255.0;
    Rgba {
        r: channel(16),
        g: channel(8),
        b: channel(0),
        a,
    }
}

fn mix(from: Rgba, to: Rgba, t: f32) -> Rgba {
    Rgba {
        r: from.r + (to.r - from.r) * t,
        g: from.g + (to.g - from.g) * t,
        b: from.b + (to.b - from.b) * t,
        a: from.a + (to.a - from.a) * t,
    }
}

impl Rgba {
    fn color(self) -> Color {
        let c = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        Color::from_rgba(c(self.r), c(self.g), c(self.b), c(self.a)).unwrap_or(Color::TRANSPARENT)
    }
}

fn rect_path(b: Bounds) -> Option<Path> {
    Rect::from_ltrb(b.left, b.top, b.right, b.bottom).map(PathBuilder::from_rect)
}

/// A rounded rectangle; the radius is limited to half the shorter side, like Direct2D.
fn rounded_path(b: Bounds, radius: f32) -> Option<Path> {
    let r = radius.min((b.right - b.left) / 2.0).min((b.bottom - b.top) / 2.0).max(0.0);
    if r <= 0.0 {
        return rect_path(b);
    }
    // The cubic approximation of a quarter circle.
    let k = r * 0.552_284_8;
    let mut p = PathBuilder::new();
    p.move_to(b.left + r, b.top);
    p.line_to(b.right - r, b.top);
    p.cubic_to(b.right - r + k, b.top, b.right, b.top + r - k, b.right, b.top + r);
    p.line_to(b.right, b.bottom - r);
    p.cubic_to(b.right, b.bottom - r + k, b.right - r + k, b.bottom, b.right - r, b.bottom);
    p.line_to(b.left + r, b.bottom);
    p.cubic_to(b.left + r - k, b.bottom, b.left, b.bottom - r + k, b.left, b.bottom - r);
    p.line_to(b.left, b.top + r);
    p.cubic_to(b.left, b.top + r - k, b.left + r - k, b.top, b.left + r, b.top);
    p.close();
    p.finish()
}

fn ellipse_path(x: f32, y: f32, rx: f32, ry: f32) -> Option<Path> {
    Rect::from_ltrb(x - rx, y - ry, x + rx, y + ry).and_then(PathBuilder::from_oval)
}

fn solid(color: Rgba) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color.color());
    paint.anti_alias = true;
    paint
}

/// The frame being drawn: the pixmap, the panel's transform and what text and icons need.
struct Canvas<'a> {
    pixmap: PixmapMut<'a>,
    /// Device pixels from panel DIPs: the monitor's scale after the panel's motion.
    transform: Transform,
    /// Device pixels from window DIPs, for the icons' placement.
    scale: f32,
    panel: Transform,
    fonts: &'a Fonts,
    icons: &'a mut HashMap<(&'static str, u32, u32), resvg::usvg::Tree>,
    clip: Option<Mask>,
}

impl Canvas<'_> {
    fn fill(&mut self, path: Option<Path>, paint: &Paint) {
        if let Some(path) = path {
            self.pixmap.fill_path(
                &path,
                paint,
                FillRule::Winding,
                self.transform,
                self.clip.as_ref(),
            );
        }
    }

    fn stroke(&mut self, path: Option<Path>, color: Rgba, width: f32) {
        if let Some(path) = path {
            let stroke = Stroke {
                width,
                ..Stroke::default()
            };
            self.pixmap.stroke_path(
                &path,
                &solid(color),
                &stroke,
                self.transform,
                self.clip.as_ref(),
            );
        }
    }

    fn fill_rect(&mut self, b: Bounds, color: Rgba) {
        self.fill(rect_path(b), &solid(color));
    }

    fn draw_rect(&mut self, b: Bounds, color: Rgba, width: f32) {
        self.stroke(rect_path(b), color, width);
    }

    fn fill_rounded(&mut self, b: Bounds, radius: f32, color: Rgba) {
        self.fill(rounded_path(b, radius), &solid(color));
    }

    fn draw_rounded(&mut self, b: Bounds, radius: f32, color: Rgba, width: f32) {
        self.stroke(rounded_path(b, radius), color, width);
    }

    fn fill_ellipse(&mut self, x: f32, y: f32, rx: f32, ry: f32, color: Rgba) {
        self.fill(ellipse_path(x, y, rx, ry), &solid(color));
    }

    fn line(&mut self, from: (f32, f32), to: (f32, f32), color: Rgba, width: f32) {
        let mut p = PathBuilder::new();
        p.move_to(from.0, from.1);
        p.line_to(to.0, to.1);
        self.stroke(p.finish(), color, width);
    }

    fn text(&mut self, text: &str, format: Format, area: Bounds, color: Rgba) {
        self.fonts.draw(
            &mut self.pixmap,
            self.transform,
            text,
            format,
            0.0,
            area,
            color.color(),
        );
    }

    fn hud_text(&mut self, text: &str, format: Format, area: Bounds, color: Rgba) {
        self.fonts.draw(
            &mut self.pixmap,
            self.transform,
            &text.to_uppercase(),
            format,
            HUD_SPACING,
            area,
            color.color(),
        );
    }

    /// A raised pill with a soft shadow and a hairline border.
    fn card(&mut self, b: Bounds, radius: f32) {
        for ring in 0..6 {
            let grow = ring as f32 * 3.0;
            let spread = bounds(
                b.left - grow,
                b.top - grow + 6.0,
                b.right + grow,
                b.bottom + grow + 6.0,
            );
            self.fill_rounded(
                spread,
                radius + grow,
                rgba(BLACK, 0.09 * (1.0 - ring as f32 / 6.0)),
            );
        }
        self.fill_rounded(b, radius, rgba(BG_RAISED, 0.96));
        self.draw_rounded(b, radius, rgba(INK, 0.15), 1.0);
    }

    /// A Lucide icon centered on `(x, y)` in panel coordinates.
    fn icon(&mut self, id: &str, color: u32, opacity: f32, x: f32, y: f32, size: f32) {
        let Some(&(key, body)) = ICONS
            .iter()
            .find(|(known, _)| *known == id)
            .or(ICONS.first())
        else {
            return;
        };
        // Quantized so a fade reuses a handful of parsed icons.
        let alpha = (opacity.clamp(0.0, 1.0) * 16.0).round() as u32;
        let cache = (key, color << 8 | alpha, size as u32);
        if !self.icons.contains_key(&cache) {
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 24 24" fill="none" stroke="#{color:06x}" stroke-opacity="{}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"##,
                alpha as f32 / 16.0
            );
            match resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()) {
                Ok(tree) => {
                    if self.icons.len() > 64 {
                        self.icons.clear();
                    }
                    self.icons.insert(cache, tree);
                }
                Err(error) => {
                    tracing::debug!(%error, id, "Popup icon not parsed");
                    return;
                }
            }
        }
        if let Some(tree) = self.icons.get(&cache) {
            let (left, top) = (x - size / 2.0, y - size / 2.0);
            let p = self.panel;
            let at = Transform::from_row(
                p.sx,
                0.0,
                0.0,
                p.sy,
                p.sx * left + p.tx,
                p.sy * top + p.ty,
            )
            .post_scale(self.scale, self.scale);
            resvg::render(tree, at, &mut self.pixmap);
        }
    }
}

/// tiny-skia over a pixmap of the window, in device pixels.
pub(super) struct Painter {
    pub(super) scale: f32,
    /// The window in DIPs; `width` × `height` in pixels.
    pub(super) size: (f32, f32),
    pub(super) width: u32,
    pub(super) height: u32,
    /// The panel's transform for this frame, in window DIPs.
    pub(super) panel: Transform,
    pixmap: Option<Pixmap>,
    icons: HashMap<(&'static str, u32, u32), resvg::usvg::Tree>,
    fonts: Fonts,
}

impl Painter {
    pub(super) fn new() -> Option<Self> {
        Some(Self {
            scale: 0.0,
            size: (0.0, 0.0),
            width: 0,
            height: 0,
            panel: Transform::identity(),
            pixmap: None,
            icons: HashMap::new(),
            fonts: Fonts::new()?,
        })
    }

    /// A pixmap for a window of `size` DIPs at `scale`; kept while neither changes.
    pub(super) fn resize(&mut self, scale: f32, size: (f32, f32)) -> bool {
        if scale == self.scale && size == self.size && self.pixmap.is_some() {
            return true;
        }
        let width = (size.0 * scale).ceil() as u32;
        let height = (size.1 * scale).ceil() as u32;
        self.pixmap = Pixmap::new(width, height);
        self.scale = scale;
        self.size = size;
        self.width = width;
        self.height = height;
        self.pixmap.is_some()
    }

    /// Frees the pixmap while nothing is shown.
    pub(super) fn release(&mut self) {
        self.pixmap = None;
        self.scale = 0.0;
    }

    /// The last frame: premultiplied RGBA, `width` × `height`.
    pub(super) fn pixels(&self) -> Option<&[u8]> {
        self.pixmap.as_ref().map(Pixmap::data)
    }

    pub(super) fn status_width(&self, title: &str, value: &str, look: OsdLook) -> f32 {
        let fonts = &self.fonts;
        let text = if look == OsdLook::Sketch {
            fonts
                .width(&title.to_uppercase(), HUD, HUD_SPACING)
                .max(fonts.width(value, MONO, 0.0))
        } else {
            fonts
                .width(title, TITLE, 0.0)
                .max(fonts.width(value, VALUE, 0.0))
        };
        (64.0 + text + 24.0).clamp(STATUS_MIN_W, STATUS_MAX_W)
    }

    pub(super) fn paint(&mut self, scene: &Scene, layout: Layout, backdrop: Option<u32>) {
        let Some(pixmap) = self.pixmap.as_mut() else {
            return;
        };
        pixmap.fill(backdrop.map_or(Color::TRANSPARENT, |hex| rgba(hex, 1.0).color()));
        let horizontal = layout.setup.style == OsdStyle::Horizontal;
        let panel = panel_size(scene.view, layout.setup.style, scene.status_width);
        let (bounds, _, _) = layout.place(self.size, panel);
        let (scale, dx, dy) = layout.transform(self.size, panel, scene.enter);
        self.panel = Transform::from_row(scale, 0.0, 0.0, scale, dx, dy);
        let mut canvas = Canvas {
            pixmap: pixmap.as_mut(),
            transform: self.panel.post_scale(self.scale, self.scale),
            scale: self.scale,
            panel: self.panel,
            fonts: &self.fonts,
            icons: &mut self.icons,
            clip: None,
        };
        if let Osd::Dictation(view) = scene.view {
            dictation(&mut canvas, scene, bounds, view);
        } else if layout.setup.look == OsdLook::Sketch {
            sketch(&mut canvas, scene, bounds, horizontal);
        } else {
            match scene.view {
                Osd::Dictation(view) => dictation(&mut canvas, scene, bounds, view),
                Osd::Volume {
                    name, color, icon, ..
                } if horizontal => bar(&mut canvas, scene, bounds, name, color, icon),
                Osd::Volume {
                    name, color, icon, ..
                } => volume(&mut canvas, scene, bounds, name, color, icon),
                Osd::Status { .. } => {
                    let radius = if horizontal {
                        BAR_RADIUS
                    } else {
                        STATUS_H / 2.0
                    };
                    status(&mut canvas, scene, bounds, radius);
                }
            }
        }
    }

    /// Previews and tests: the current frame as a PNG.
    #[cfg(test)]
    pub(super) fn save_png(&self, path: &std::path::Path) {
        if let Some(pixmap) = &self.pixmap {
            pixmap.save_png(path).expect("PNG");
        }
    }
}

/// Square frame and ruler, using the same animated scene as Classic.
fn sketch(c: &mut Canvas, scene: &Scene, b: Bounds, horizontal: bool) {
    let positive = scene.stretch.max(0.0) * STRETCH / KICK * 4.0;
    let negative = (-scene.stretch).max(0.0) * STRETCH / KICK * 4.0;
    let b = if matches!(scene.view, Osd::Volume { .. }) {
        if horizontal {
            bounds(b.left - negative, b.top, b.right + positive, b.bottom)
        } else {
            bounds(b.left, b.top - positive, b.right, b.bottom + negative)
        }
    } else {
        b
    };
    let line = rgba(INK, 0.15);
    let strong = rgba(INK, 0.4);
    c.fill_rect(b, rgba(BG_RAISED, 1.0));
    c.draw_rect(b, line, 1.0);
    for x in [b.left, b.right] {
        for y in [b.top, b.bottom] {
            c.line((x - 5.5, y), (x + 5.5, y), strong, 1.0);
            c.line((x, y - 5.5), (x, y + 5.5), strong, 1.0);
        }
    }
    match scene.view {
        Osd::Dictation(view) => dictation(c, scene, b, view),
        Osd::Volume {
            name, color, icon, ..
        } => {
            let (fill, ink) = channel_colors(color);
            let track = if horizontal {
                bounds(b.left + 54.0, b.top + 34.0, b.right - 64.0, b.bottom - 12.0)
            } else {
                bounds(b.left + 14.0, b.top + 46.0, b.right - 14.0, b.bottom - 46.0)
            };
            c.fill_rect(track, rgba(SUNKEN, 1.0));
            let level = scene.level.clamp(0.0, 1.0);
            let head = if horizontal {
                track.left + (track.right - track.left) * level
            } else {
                track.bottom - (track.bottom - track.top) * level
            };
            let filled = if horizontal {
                bounds(track.left, track.top, head, track.bottom)
            } else {
                bounds(track.left, head, track.right, track.bottom)
            };
            c.fill_rect(filled, rgba(fill, 1.0 - 0.75 * scene.mute));
            c.draw_rect(track, line, 1.0);
            for tick in 0..=10 {
                let t = tick as f32 / 10.0;
                let length = if tick % 5 == 0 { 7.0 } else { 4.0 };
                let (from, to) = if horizontal {
                    let x = track.left + (track.right - track.left) * t;
                    ((x, track.bottom), (x, track.bottom - length))
                } else {
                    let y = track.bottom - (track.bottom - track.top) * t;
                    ((track.left, y), (track.left + length, y))
                };
                c.line(from, to, strong, 1.0);
            }
            let head_color = rgba(ink, 1.0 - 0.5 * scene.mute);
            let (from, to) = if horizontal {
                ((head, track.top - 3.0), (head, track.bottom + 3.0))
            } else {
                ((track.left - 3.0, head), (track.right + 3.0, head))
            };
            c.line(from, to, head_color, 2.0);
            let (caption, value, caption_format) = if horizontal {
                c.icon(
                    icon,
                    ink,
                    1.0 - 0.4 * scene.mute,
                    b.left + 27.0,
                    (b.top + b.bottom) / 2.0,
                    20.0,
                );
                (
                    bounds(track.left, b.top + 8.0, track.right, b.top + 29.0),
                    bounds(b.right - 60.0, b.top, b.right - 8.0, b.bottom),
                    HUD,
                )
            } else {
                (
                    bounds(b.left + 4.0, b.bottom - 38.0, b.right - 4.0, b.bottom - 12.0),
                    bounds(b.left + 4.0, b.top + 12.0, b.right - 4.0, b.top + 36.0),
                    HUD_CENTER,
                )
            };
            c.hud_text(name, caption_format, caption, rgba(ink, 1.0));
            c.text(
                &format!("{:.0}%", level * 100.0),
                MONO_CENTER,
                value,
                mix(rgba(INK, 1.0), rgba(DANGER, 1.0), scene.mute),
            );
        }
        Osd::Status {
            icon, title, value, ..
        } => {
            let x = b.left + STATUS_H / 2.0;
            let y = (b.top + b.bottom) / 2.0;
            c.fill_rect(
                bounds(x - 20.0, y - 20.0, x + 20.0, y + 20.0),
                mix(rgba(ACCENT, 1.0), rgba(DANGER, 1.0), scene.alert),
            );
            c.icon(status_icon(*icon), WHITE, 1.0, x, y, 22.0);
            c.hud_text(
                title,
                HUD,
                bounds(b.left + STATUS_H, b.top + 9.0, b.right - 16.0, b.top + 30.0),
                rgba(INK, 0.6),
            );
            c.text(
                value,
                MONO,
                bounds(b.left + STATUS_H, b.top + 29.0, b.right - 16.0, b.bottom - 8.0),
                rgba(INK, 1.0),
            );
        }
    }
}

/// Android-like: a pill-shaped slider in the channel's color with its icon and name.
fn volume(c: &mut Canvas, scene: &Scene, b: Bounds, name: &str, color: &str, icon: &str) {
    let up = scene.stretch.max(0.0) * STRETCH / KICK * 4.0;
    let down = (-scene.stretch).max(0.0) * STRETCH / KICK * 4.0;
    let b = bounds(b.left, b.top - up, b.right, b.bottom + down);
    let (fill, ink) = channel_colors(color);
    let grey = rgba(INK, 0.32);
    c.card(b, VOLUME_W / 2.0);
    // What it is set to: the percentage, crossfading to "Muted".
    let label = bounds(b.left, b.top + 12.0, b.right, b.top + 38.0);
    let percent = format!("{:.0}%", scene.level * 100.0);
    c.text(&percent, PERCENT, label, rgba(INK, 1.0 - scene.mute));
    c.text("Muted", PERCENT, label, rgba(DANGER, scene.mute));
    // The slider: an empty track and a fill that keeps at least the icon's circle.
    let track = bounds(b.left + 10.0, b.top + 46.0, b.right - 10.0, b.bottom - 44.0);
    let thickness = track.right - track.left;
    c.fill_rounded(track, thickness / 2.0, rgba(INK, 0.10));
    let length = thickness + (track.bottom - track.top - thickness) * scene.level;
    let level = bounds(track.left, track.bottom - length, track.right, track.bottom);
    let base = mix(rgba(fill, 1.0), grey, scene.mute);
    let light = mix(mix(rgba(fill, 1.0), rgba(ink, 1.0), 0.45), grey, scene.mute);
    if let Some(gradient) = LinearGradient::new(
        Point::from_xy(0.0, level.top),
        Point::from_xy(0.0, level.bottom),
        vec![
            GradientStop::new(0.0, light.color()),
            GradientStop::new(1.0, base.color()),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    ) {
        let paint = Paint {
            shader: gradient,
            anti_alias: true,
            ..Paint::default()
        };
        c.fill(rounded_path(level, thickness / 2.0), &paint);
    } else {
        // A level too short for a gradient: its base color.
        c.fill_rounded(level, thickness / 2.0, base);
    }
    // Which channel: its icon in the fill and its name in its color.
    let center_x = (track.left + track.right) / 2.0;
    c.icon(
        icon,
        WHITE,
        1.0 - 0.4 * scene.mute,
        center_x,
        track.bottom - thickness / 2.0,
        22.0,
    );
    let caption = bounds(b.left + 6.0, b.bottom - 38.0, b.right - 6.0, b.bottom - 14.0);
    c.text(name, NAME, caption, rgba(ink, 1.0));
}

fn dictation(c: &mut Canvas, scene: &Scene, b: Bounds, view: &DictationView) {
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
    let ink = rgba(INK, 1.0);
    if view.look == DictationPopupLook::Flow {
        let radius = (b.bottom - b.top) / 2.0;
        c.card(b, radius);
        c.fill_rounded(b, radius, rgba(INVERT_FG, 0.92));
        if danger {
            c.draw_rounded(b, radius, rgba(DANGER, 1.0), 1.0);
        }
        let center_y = (b.top + b.bottom) / 2.0;
        if listening {
            // The last 30 seconds before the limit: the bars turn amber.
            let bars = if view.elapsed_s >= 270 {
                rgba(AMBER, 1.0)
            } else {
                ink
            };
            let left = if view.hands_free {
                b.left + 14.0
            } else {
                (b.left + b.right - FLOW_BARS_W) / 2.0
            };
            for n in 0..15 {
                let energy = view.bands[n.min(14 - n)];
                let mirrored = 1.0 - ((n as f32 - 7.0) / 8.0).abs() * 0.25;
                let height = 3.0 + energy.clamp(0.0, 1.0) * 18.0 * mirrored;
                let x = left + n as f32 * 6.0;
                c.fill_rounded(
                    bounds(x, center_y - height / 2.0, x + 3.0, center_y + height / 2.0),
                    1.5,
                    bars,
                );
            }
        } else if transcribing {
            for n in 0..3 {
                let phase =
                    (scene.shimmer * std::f32::consts::TAU / 0.9 - n as f32 * 0.8).sin() * 0.5 + 0.5;
                c.fill_ellipse(
                    (b.left + b.right) / 2.0 + (n as f32 - 1.0) * 14.0,
                    center_y,
                    3.0,
                    3.0,
                    rgba(INK, 0.3 + 0.7 * phase),
                );
            }
        } else if matches!(view.state, DictationState::Done | DictationState::Copied) {
            let x = b.left
                + if view.state == DictationState::Done {
                    10.0
                } else {
                    18.0
                };
            c.line((x, center_y), (x + 5.0, center_y + 5.0), ink, 2.0);
            c.line((x + 5.0, center_y + 5.0), (x + 15.0, center_y - 7.0), ink, 2.0);
            if view.state == DictationState::Copied {
                c.text(
                    "Copied",
                    DICTATION_BODY,
                    bounds(x + 26.0, b.top, b.right - 16.0, b.bottom),
                    ink,
                );
            }
        } else {
            c.hud_text(
                caption,
                DICTATION_HUD,
                bounds(b.left + 20.0, b.top, b.right - 20.0, b.bottom),
                rgba(if danger { DANGER } else { INK }, 1.0),
            );
        }
    } else {
        analog(c, scene, b, view, caption);
    }
    if view.hands_free && listening {
        let y = if view.look == DictationPopupLook::Flow {
            (b.top + b.bottom) / 2.0 - 3.0
        } else {
            b.top + 8.0
        };
        c.fill_rect(bounds(b.right - 39.0, y, b.right - 33.0, y + 6.0), ink);
        c.line((b.right - 21.0, y), (b.right - 15.0, y + 6.0), ink, 1.0);
        c.line((b.right - 15.0, y), (b.right - 21.0, y + 6.0), ink, 1.0);
    }
}

/// The Analog look: a thin arc with the red zone, a white needle with VU ballistics and its
/// slowly falling peak ghost, a lamp glow that breathes with the voice; while transcribing the
/// needle sways and a light runs along the arc.
fn analog(c: &mut Canvas, scene: &Scene, b: Bounds, view: &DictationView, caption: &str) {
    let listening = view.state == DictationState::Listening;
    let transcribing = view.state == DictationState::Transcribing;
    let danger = view.state == DictationState::Error;
    let done = matches!(view.state, DictationState::Done | DictationState::Copied);
    c.fill_rect(b, rgba(BG_RAISED, 1.0));
    c.draw_rect(b, rgba(INK, 0.22), 1.0);
    let tick = rgba(INK, 0.45);
    for x in [b.left, b.right] {
        for y in [b.top, b.bottom] {
            c.line((x - 4.0, y), (x + 4.0, y), tick, 1.0);
            c.line((x, y - 4.0), (x, y + 4.0), tick, 1.0);
        }
    }
    let center = ((b.left + b.right) / 2.0, b.top + 42.0);
    let radius = 32.0;
    // −50°…+50° from vertical.
    let point = |travel: f32, r: f32| {
        let a = (-50.0 + 100.0 * travel).to_radians();
        (center.0 + a.sin() * r, center.1 - a.cos() * r)
    };
    let level = vu_travel(scene.vu).clamp(0.0, 1.0);
    // The lamp: an amber glow under the arc that breathes with the voice.
    if listening || transcribing {
        let mut mask = Mask::new(c.pixmap.width(), c.pixmap.height());
        if let (Some(mask), Some(path)) = (mask.as_mut(), rect_path(b)) {
            mask.fill_path(&path, FillRule::Winding, true, c.transform);
        }
        c.clip = mask;
        let strength = if listening { 0.05 + 0.16 * level } else { 0.04 };
        for (r, a) in [(38.0, 0.35), (26.0, 0.6), (15.0, 1.0)] {
            c.fill_ellipse(center.0, center.1, r, r * 0.8, rgba(AMBER_DEEP, strength * a));
        }
        c.clip = None;
    }
    let red_from = vu_travel(1.0);
    let segments = 48;
    let faint = rgba(INK, 0.38);
    let red = rgba(DANGER, 1.0);
    for n in 0..segments {
        let (from, to) = (n as f32 / segments as f32, (n + 1) as f32 / segments as f32);
        let hot = from >= red_from - 0.001;
        c.line(
            point(from, radius),
            point(to, radius),
            if hot { red } else { faint },
            if hot { 2.0 } else { 1.0 },
        );
    }
    for vu in [-20.0f32, -10.0, -5.0, 0.0, 3.0] {
        let travel = vu_travel(10f32.powf(vu / 20.0));
        c.line(
            point(travel, radius - 4.0),
            point(travel, radius),
            if vu >= 0.0 { red } else { faint },
            1.0,
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
            let (x, y) = point(at, radius);
            c.fill_ellipse(x, y, 1.6, 1.6, rgba(AMBER, 1.0 - n as f32 / 6.0));
        }
    }
    c.text(
        "VU",
        DICTATION_SERIF,
        bounds(center.0 - 30.0, center.1 - 16.0, center.0 - 10.0, center.1 - 1.0),
        rgba(INK, 0.35),
    );
    // Peak ghost, then the needle.
    let needle_travel = if transcribing {
        0.18 + 0.1 * (scene.shimmer * 2.2).sin()
    } else {
        vu_travel(scene.vu)
    };
    if listening && scene.vu_peak > needle_travel + 0.02 {
        c.line(center, point(scene.vu_peak, radius - 3.0), rgba(INK, 0.22), 1.0);
    }
    let white = rgba(WHITE, if done { 0.7 } else { 1.0 });
    c.line(center, point(needle_travel, radius - 3.0), white, 1.5);
    c.fill_ellipse(center.0, center.1, 2.5, 2.5, white);
    // The peak lamp lights on clipping.
    if listening && view.rms_dbfs > -7.0 {
        c.fill_rect(
            bounds(b.right - 12.0, b.top + 8.0, b.right - 8.0, b.top + 12.0),
            red,
        );
    }
    // HUD line: a blinking REC dot while listening, the state otherwise; no timer and no
    // language, only the voice.
    let hud_top = b.top + 47.0;
    if listening {
        let on = scene.shimmer.fract() < 0.6;
        let dot = if view.elapsed_s >= 270 { AMBER } else { DANGER };
        c.fill_ellipse(
            b.left + 10.5,
            hud_top + 5.5,
            2.0,
            2.0,
            rgba(dot, if on { 1.0 } else { 0.25 }),
        );
    } else {
        let label = match view.state {
            DictationState::Done => format!("TYPED · {}", view.words),
            _ => caption.to_owned(),
        };
        c.hud_text(
            &label,
            DICTATION_HUD,
            bounds(b.left + 8.0, hud_top, b.right - 8.0, b.bottom - 2.0),
            rgba(if danger { DANGER } else { INK }, 0.85),
        );
    }
}

fn status(c: &mut Canvas, scene: &Scene, b: Bounds, radius: f32) {
    let Osd::Status {
        icon, title, value, ..
    } = scene.view
    else {
        return;
    };
    c.card(b, radius);
    let center = (b.left + STATUS_H / 2.0, (b.top + b.bottom) / 2.0);
    c.fill_ellipse(
        center.0,
        center.1,
        22.0,
        22.0,
        mix(rgba(ACCENT, 1.0), rgba(DANGER, 1.0), scene.alert),
    );
    c.icon(status_icon(*icon), WHITE, 1.0, center.0, center.1, 22.0);
    let left = b.left + STATUS_H;
    c.text(
        title,
        TITLE,
        bounds(left, b.top + 11.0, b.right - 20.0, b.top + 31.0),
        rgba(INK, 0.6),
    );
    c.text(
        value,
        VALUE,
        bounds(left, b.top + 28.0, b.right - 20.0, b.top + 54.0),
        rgba(INK, 1.0),
    );
}

/// Windows 11-like: the channel's icon, its name over a thin slider with a round thumb, the
/// percentage at the end.
fn bar(c: &mut Canvas, scene: &Scene, b: Bounds, name: &str, color: &str, icon: &str) {
    let right = scene.stretch.max(0.0) * STRETCH / KICK * 4.0;
    let left = (-scene.stretch).max(0.0) * STRETCH / KICK * 4.0;
    let b = bounds(b.left - left, b.top, b.right + right, b.bottom);
    let (fill, ink) = channel_colors(color);
    let grey = rgba(INK, 0.32);
    let middle = (b.top + b.bottom) / 2.0;
    c.card(b, BAR_RADIUS);
    c.icon(icon, ink, 1.0 - 0.5 * scene.mute, b.left + 30.0, middle, 22.0);
    let (start, end) = (b.left + 56.0, b.right - 74.0);
    let caption = bounds(start, b.top + 9.0, end, b.top + 27.0);
    c.text(name, LABEL, caption, rgba(ink, 1.0));
    let y = b.top + 40.0;
    c.fill_rounded(bounds(start, y - 2.0, end, y + 2.0), 2.0, rgba(INK, 0.18));
    let x = start + (end - start) * scene.level;
    let filled = mix(rgba(fill, 1.0), grey, scene.mute);
    c.fill_rounded(bounds(start, y - 2.0, x.max(start + 4.0), y + 2.0), 2.0, filled);
    let ring = mix(rgba(BG_RAISED, 1.0), rgba(INK, 1.0), 0.22);
    c.fill_ellipse(x, y, 10.0, 10.0, ring);
    c.fill_ellipse(x, y, 6.0, 6.0, filled);
    let number = bounds(b.right - 60.0, b.top, b.right - 10.0, b.bottom);
    let percent = format!("{:.0}%", scene.level * 100.0);
    c.text(&percent, PERCENT, number, rgba(INK, 1.0 - scene.mute));
    c.text("Muted", PERCENT, number, rgba(DANGER, scene.mute));
}

#[cfg(test)]
mod tests {
    use super::super::super::StatusIcon;
    use super::super::super::dictation::State;
    use super::*;
    use macafon_config::{OsdConfig, OsdEdge};

    fn views() -> Vec<(&'static str, Osd)> {
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
            Osd::Dictation(DictationView {
                look,
                state,
                rms_dbfs: -20.0,
                bands: std::array::from_fn(|n| 0.3 + (n as f32 * 0.8).sin().abs() * 0.6),
                elapsed_s: 7,
                engine: "WHISPER".into(),
                words: 14,
                language: "EN".into(),
                hands_free,
            })
        };
        use macafon_ipc::DictationPopupLook::{Analog, Flow};
        vec![
            ("music", volume("Music", "pink", "music", 67.0, false)),
            ("voice-muted", volume("Voice", "violet", "message", 40.0, true)),
            ("mic-muted", status(StatusIcon::MicOff, "Microphone", "Muted", true)),
            ("bypass", status(StatusIcon::Processing, "Processing", "Bypassed", true)),
            ("preset", status(StatusIcon::Preset, "Microphone preset", "Natural", false)),
            ("display", status(StatusIcon::Monitor, "Display", "Work", false)),
            ("flow-listening", dictation(Flow, State::Listening, true)),
            ("flow-transcribing", dictation(Flow, State::Transcribing, false)),
            ("flow-copied", dictation(Flow, State::Copied, false)),
            ("flow-error", dictation(Flow, State::Error, false)),
            ("analog-listening", dictation(Analog, State::Listening, false)),
            ("analog-done", dictation(Analog, State::Done, false)),
        ]
    }

    fn scene<'a>(painter: &Painter, view: &'a Osd, look: OsdLook) -> Scene<'a> {
        let (level, mute, alert, status_width) = match view {
            Osd::Volume {
                volume_pct, muted, ..
            } => (volume_pct / 100.0, f32::from(u8::from(*muted)), 0.0, STATUS_MIN_W),
            Osd::Status {
                title,
                value,
                alert,
                ..
            } => (
                0.0,
                0.0,
                f32::from(u8::from(*alert)),
                painter.status_width(title, value, look),
            ),
            Osd::Dictation(_) => (0.0, 0.0, 0.0, STATUS_MIN_W),
        };
        Scene {
            view,
            enter: 1.0,
            level,
            mute,
            alert,
            stretch: 0.0,
            status_width,
            vu: 0.8,
            vu_peak: 0.9,
            shimmer: 0.5,
        }
    }

    /// Every look, style and state paints its panel, and only inside the panel's bounds plus
    /// the shadow.
    #[test]
    fn every_popup_paints_inside_its_panel() {
        let mut painter = Painter::new().expect("fonts");
        for look in [OsdLook::Classic, OsdLook::Sketch] {
            for (style, edge) in [
                (OsdStyle::Vertical, OsdEdge::Right),
                (OsdStyle::Horizontal, OsdEdge::Bottom),
            ] {
                let setup = OsdConfig {
                    look,
                    style,
                    edge,
                    ..OsdConfig::default()
                };
                let layout = Layout::new(setup, MARGIN, 0.0);
                assert!(painter.resize(1.5, layout.window()));
                for (label, view) in views() {
                    let scene = scene(&painter, &view, look);
                    painter.paint(&scene, layout, None);
                    let panel = panel_size(&view, style, scene.status_width);
                    let (b, _, _) = layout.place(painter.size, panel);
                    let pixels = painter.pixels().expect("pixmap");
                    let (mut inside, mut outside) = (0, 0);
                    for (n, pixel) in pixels.chunks_exact(4).enumerate() {
                        if pixel[3] == 0 {
                            continue;
                        }
                        let x = (n as u32 % painter.width) as f32 / 1.5;
                        let y = (n as u32 / painter.width) as f32 / 1.5;
                        let margin = SHADOW + STRETCH;
                        if x >= b.left - margin
                            && x <= b.right + margin
                            && y >= b.top - margin
                            && y <= b.bottom + margin
                        {
                            inside += 1;
                        } else {
                            outside += 1;
                        }
                    }
                    let area = ((b.right - b.left) * (b.bottom - b.top) * 2.25) as usize;
                    assert!(inside > area / 2, "{look:?} {style:?} {label}: {inside} of {area}");
                    assert_eq!(outside, 0, "{look:?} {style:?} {label}");
                }
            }
        }
    }

    #[test]
    fn status_icons_exist() {
        for icon in [
            StatusIcon::Mic,
            StatusIcon::MicOff,
            StatusIcon::Processing,
            StatusIcon::Preset,
            StatusIcon::Monitor,
        ] {
            assert!(ICONS.iter().any(|(id, _)| *id == status_icon(icon)), "{icon:?}");
        }
    }

    /// `cargo test -p macafond linux_osd_previews -- --ignored` writes PNGs for review.
    #[test]
    #[ignore = "writes preview images"]
    fn linux_osd_previews() {
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.artifacts/screenshots");
        std::fs::create_dir_all(&directory).expect("preview directory");
        let mut painter = Painter::new().expect("fonts");
        for (look_name, look) in [("classic", OsdLook::Classic), ("sketch", OsdLook::Sketch)] {
            for (style_name, style, edge) in [
                ("vertical", OsdStyle::Vertical, OsdEdge::Right),
                ("horizontal", OsdStyle::Horizontal, OsdEdge::Bottom),
            ] {
                let setup = OsdConfig {
                    look,
                    style,
                    edge,
                    ..OsdConfig::default()
                };
                let layout = Layout::new(setup, MARGIN, 0.0);
                painter.resize(2.0, layout.window());
                for (label, view) in views() {
                    let scene = scene(&painter, &view, look);
                    painter.paint(&scene, layout, Some(0x3a4a5c));
                    painter.save_png(&directory.join(format!(
                        "2026-10-31-linux-osd-preview-{look_name}-{style_name}-{label}.png"
                    )));
                }
            }
        }
    }
}
