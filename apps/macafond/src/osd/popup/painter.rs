//! The popup's pictures: Direct2D draws every look and state of the hotkey and dictation
//! popups into a WIC bitmap, which `present` copies into the layered window's DIB.
use super::*;

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

pub(super) fn rgba(hex: u32, a: f32) -> D2D1_COLOR_F {
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

pub(super) fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
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

pub(super) const IDENTITY: Matrix3x2 = Matrix3x2 {
    M11: 1.0,
    M12: 0.0,
    M21: 0.0,
    M22: 1.0,
    M31: 0.0,
    M32: 0.0,
};

/// Direct2D over a WIC bitmap plus the GDI DIB `UpdateLayeredWindow` reads from.
pub(super) struct Painter {
    pub(super) d2d: ID2D1Factory,
    pub(super) dwrite: IDWriteFactory,
    pub(super) wic: IWICImagingFactory,
    pub(super) bitmap: Option<IWICBitmap>,
    pub(super) target: Option<ID2D1RenderTarget>,
    /// SVG icons need `ID2D1DeviceContext5` (Windows 10 1703+); without it icons are skipped.
    pub(super) context: Option<ID2D1DeviceContext5>,
    pub(super) dib: HBITMAP,
    pub(super) dc: HDC,
    pub(super) bits: *mut u8,
    pub(super) scale: f32,
    /// The window in DIPs; `width` × `height` in pixels.
    pub(super) size: (f32, f32),
    pub(super) width: u32,
    pub(super) height: u32,
    /// The panel's transform for this frame: icons are placed relative to it.
    pub(super) panel: Matrix3x2,
    pub(super) icons: HashMap<(&'static str, u32, u32), ID2D1SvgDocument>,
    pub(super) mono: IDWriteTextFormat,
    pub(super) mono_center: IDWriteTextFormat,
    pub(super) hud: IDWriteTextFormat,
    pub(super) hud_center: IDWriteTextFormat,
    pub(super) percent: IDWriteTextFormat,
    pub(super) name: IDWriteTextFormat,
    pub(super) label: IDWriteTextFormat,
    pub(super) title: IDWriteTextFormat,
    pub(super) value: IDWriteTextFormat,
    pub(super) dictation_hud: IDWriteTextFormat,
    pub(super) dictation_hud_center: IDWriteTextFormat,
    pub(super) dictation_serif: IDWriteTextFormat,
    pub(super) dictation_body: IDWriteTextFormat,
    pub(super) _fonts: crate::osd::fonts::Fonts,
}

impl Painter {
    pub(super) fn new() -> Result<Self> {
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
            let fonts = crate::osd::fonts::Fonts::new(&dwrite)?;
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
    pub(super) fn resize(&mut self, scale: f32, size: (f32, f32)) -> Result<()> {
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

    pub(super) fn status_width(&self, title: &str, value: &str, look: OsdLook) -> f32 {
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

    pub(super) fn paint(
        &mut self,
        scene: &Scene,
        layout: Layout,
        backdrop: Option<u32>,
    ) -> Result<()> {
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
    pub(super) fn present(&self, window: HWND, origin: POINT, opacity: f32) -> Result<()> {
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
    pub(super) fn save_png(&self, path: &std::path::Path) -> Result<()> {
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
