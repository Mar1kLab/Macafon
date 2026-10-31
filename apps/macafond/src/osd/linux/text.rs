//! Text for the software painter: the UI's bundled OFL fonts (Inter, Geist Mono, Instrument
//! Serif; TrueType copies of the WOFF2 files the Windows popup unpacks), shaped by rustybuzz and
//! filled as tiny-skia paths. Each format mirrors a DirectWrite one of the Windows painter: one
//! line, vertically centered in its box, leading or centered, trimmed with an ellipsis.
use super::super::motion::Bounds;
use rustybuzz::ttf_parser::{GlyphId, OutlineBuilder, Tag};
use rustybuzz::{Face, UnicodeBuffer, Variation};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, PixmapMut, Transform};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Family {
    /// Geist Mono 500: HUD labels and values.
    Mono,
    /// Inter 400: titles and body text.
    Sans,
    /// Inter 600: what Segoe UI Variable Semibold shows on Windows.
    SansStrong,
    /// Instrument Serif italic: the Analog meter's "VU".
    Serif,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Format {
    pub(super) family: Family,
    pub(super) size: f32,
    pub(super) centered: bool,
}

pub(super) const fn format(family: Family, size: f32, centered: bool) -> Format {
    Format {
        family,
        size,
        centered,
    }
}

pub(super) struct Fonts {
    mono: Face<'static>,
    sans: Face<'static>,
    sans_strong: Face<'static>,
    serif: Face<'static>,
}

fn face(data: &'static [u8], weight: Option<f32>) -> Option<Face<'static>> {
    let mut face = Face::from_slice(data, 0)?;
    if let Some(value) = weight {
        face.set_variations(&[Variation {
            tag: Tag::from_bytes(b"wght"),
            value,
        }]);
    }
    Some(face)
}

/// Glyphs of one line: id and pen position in DIPs from the line's start.
struct Line {
    glyphs: Vec<(GlyphId, f32, f32)>,
    width: f32,
}

impl Fonts {
    pub(super) fn new() -> Option<Self> {
        const MONO: &[u8] = include_bytes!("../../../assets/fonts/geist-mono.ttf");
        const SANS: &[u8] = include_bytes!("../../../assets/fonts/inter.ttf");
        const SERIF: &[u8] = include_bytes!("../../../assets/fonts/instrument-serif-italic.ttf");
        Some(Self {
            mono: face(MONO, Some(500.0))?,
            sans: face(SANS, Some(400.0))?,
            sans_strong: face(SANS, Some(600.0))?,
            serif: face(SERIF, None)?,
        })
    }

    fn face(&self, family: Family) -> &Face<'static> {
        match family {
            Family::Mono => &self.mono,
            Family::Sans => &self.sans,
            Family::SansStrong => &self.sans_strong,
            Family::Serif => &self.serif,
        }
    }

    /// `spacing` is added after every character, like DirectWrite's trailing spacing.
    fn shape(&self, text: &str, format: Format, spacing: f32) -> Line {
        let face = self.face(format.family);
        let em = format.size / face.units_per_em().max(1) as f32;
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(text);
        let shaped = rustybuzz::shape(face, &[], buffer);
        let mut x = 0.0;
        let glyphs = shaped
            .glyph_infos()
            .iter()
            .zip(shaped.glyph_positions())
            .map(|(info, position)| {
                let glyph = (
                    GlyphId(info.glyph_id as u16),
                    x + position.x_offset as f32 * em,
                    position.y_offset as f32 * em,
                );
                x += position.x_advance as f32 * em + spacing;
                glyph
            })
            .collect();
        Line { glyphs, width: x }
    }

    /// The advance width of `text`, trailing spacing included.
    pub(super) fn width(&self, text: &str, format: Format, spacing: f32) -> f32 {
        self.shape(text, format, spacing).width
    }

    /// `text` on one line in `area`, cut with "…" where it does not fit.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw(
        &self,
        pixmap: &mut PixmapMut,
        transform: Transform,
        text: &str,
        format: Format,
        spacing: f32,
        area: Bounds,
        color: Color,
    ) {
        if color.alpha() <= 0.0 || text.is_empty() {
            return;
        }
        let room = area.right - area.left;
        let mut line = self.shape(text, format, spacing);
        if line.width > room + 0.01 {
            // Character trimming: the longest start that fits with the ellipsis.
            let mut ends: Vec<usize> = text.char_indices().map(|(at, _)| at).collect();
            line = self.shape("…", format, spacing);
            while let Some(end) = ends.pop() {
                let candidate = self.shape(&format!("{}…", &text[..end]), format, spacing);
                if candidate.width <= room + 0.01 {
                    line = candidate;
                    break;
                }
            }
        }
        let face = self.face(format.family);
        let em = format.size / face.units_per_em().max(1) as f32;
        let (ascent, descent, gap) = (
            f32::from(face.ascender()) * em,
            f32::from(face.descender()) * em,
            f32::from(face.line_gap()) * em,
        );
        let height = ascent - descent + gap;
        let baseline = area.top + (area.bottom - area.top - height) / 2.0 + gap / 2.0 + ascent;
        let left = if format.centered {
            area.left + (room - line.width) / 2.0
        } else {
            area.left
        };
        let mut outline = Outline {
            builder: PathBuilder::new(),
            x: 0.0,
            y: 0.0,
            em,
        };
        for (glyph, x, y) in &line.glyphs {
            outline.x = left + x;
            outline.y = baseline - y;
            face.outline_glyph(*glyph, &mut outline);
        }
        let Some(path) = outline.builder.finish() else {
            return;
        };
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        pixmap.fill_path(&path, &paint, FillRule::Winding, transform, None);
    }
}

/// Glyph outlines in font units onto the line in DIPs (y grows down).
struct Outline {
    builder: PathBuilder,
    x: f32,
    y: f32,
    em: f32,
}

impl Outline {
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x * self.em, self.y - y * self.em)
    }
}

impl OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        self.builder.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        self.builder.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let ((x1, y1), (x, y)) = (self.at(x1, y1), self.at(x, y));
        self.builder.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let ((x1, y1), (x2, y2), (x, y)) = (self.at(x1, y1), self.at(x2, y2), self.at(x, y));
        self.builder.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.builder.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_widen_and_long_text_is_trimmed() {
        let fonts = Fonts::new().expect("bundled fonts");
        let regular = fonts.width("Microphone preset", format(Family::Sans, 12.0, false), 0.0);
        let strong = fonts.width("Microphone preset", format(Family::SansStrong, 12.0, false), 0.0);
        assert!(regular > 60.0 && strong > regular, "{regular} {strong}");
        // Mono: every character the same advance, plus the HUD spacing.
        let mono = format(Family::Mono, 11.0, false);
        let (one, ten) = (fonts.width("A", mono, 1.65), fonts.width("AAAAAAAAAA", mono, 1.65));
        assert!((ten - 10.0 * one).abs() < 0.01, "{one} {ten}");
        let mut pixmap = tiny_skia::Pixmap::new(80, 20).expect("pixmap");
        fonts.draw(
            &mut pixmap.as_mut(),
            Transform::identity(),
            "A channel name far too long for its box",
            format(Family::Sans, 12.0, false),
            0.0,
            super::super::super::motion::bounds(0.0, 0.0, 60.0, 20.0),
            Color::WHITE,
        );
        let inked = |x0: u32, x1: u32| {
            (0..20).any(|y| (x0..x1).any(|x| pixmap.pixel(x, y).is_some_and(|p| p.alpha() > 0)))
        };
        assert!(inked(0, 60), "text drawn");
        assert!(!inked(61, 80), "trimmed inside its box");
    }
}
