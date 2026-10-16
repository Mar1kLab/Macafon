//! Executable icons, resolved outside the audio threads.
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Shell::ExtractIconExW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

const SIZE: usize = 32;

pub(super) fn data_url(name: &str) -> Option<String> {
    let path = Path::new(name);
    if !path.is_absolute() || !path.extension()?.eq_ignore_ascii_case("exe") || name.contains('\0')
    {
        return None;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut icon = HICON::default();
    // SAFETY: the path is NUL-terminated and the output points to a live HICON slot.
    let count = unsafe { ExtractIconExW(PCWSTR(wide.as_ptr()), 0, Some(&mut icon), None, 1) };
    if count == 0 || icon.is_invalid() {
        return None;
    }
    let rgba = render(icon);
    // SAFETY: ExtractIconExW transferred ownership of this icon to us.
    unsafe {
        let _ = DestroyIcon(icon);
    }
    let rgba = rgba?;
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, SIZE as u32, SIZE as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().ok()?.write_image_data(&rgba).ok()?;
    }
    Some(format!("data:image/png;base64,{}", super::base64(&bytes)))
}

fn render(icon: HICON) -> Option<Vec<u8>> {
    // SAFETY: all GDI objects are local, selected objects are restored before deletion,
    // and the DIB is a top-down 32-bit buffer of exactly SIZE * SIZE pixels.
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return None;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: SIZE as i32,
                biHeight: -(SIZE as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0);
        let result = if let Ok(bitmap) = bitmap {
            let previous = SelectObject(dc, bitmap.into());
            let result = (|| {
                let pixels = std::slice::from_raw_parts_mut(bits.cast::<u8>(), SIZE * SIZE * 4);
                // Drawing on black and white reconstructs alpha for both legacy masked
                // icons and modern alpha icons, without trusting unused DIB alpha bytes.
                pixels.fill(0);
                DrawIconEx(dc, 0, 0, icon, SIZE as i32, SIZE as i32, 0, None, DI_NORMAL).ok()?;
                GdiFlush().ok().ok()?;
                let black = pixels.to_vec();
                pixels.fill(255);
                DrawIconEx(dc, 0, 0, icon, SIZE as i32, SIZE as i32, 0, None, DI_NORMAL).ok()?;
                GdiFlush().ok().ok()?;
                let mut rgba = vec![0; pixels.len()];
                for ((black, white), out) in black
                    .chunks_exact(4)
                    .zip(pixels.chunks_exact(4))
                    .zip(rgba.chunks_exact_mut(4))
                {
                    let alpha = 255 - white[0].saturating_sub(black[0]);
                    for (index, channel) in [2, 1, 0].into_iter().enumerate() {
                        out[index] = if alpha == 0 {
                            0
                        } else {
                            (u32::from(black[channel]) * 255 / u32::from(alpha)).min(255) as u8
                        };
                    }
                    out[3] = alpha;
                }
                Some(rgba)
            })();
            SelectObject(dc, previous);
            let _ = DeleteObject(bitmap.into());
            result
        } else {
            None
        };
        let _ = DeleteDC(dc);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_icon_is_a_cached_png_data_url() {
        let path = std::env::var_os("WINDIR").expect("Windows directory");
        let path = Path::new(&path).join("explorer.exe");
        let name = path.to_str().expect("Windows path");
        let mut resolver = super::super::IconResolver::new(Vec::new());
        let url = resolver.data_url(name).expect("executable icon");
        assert!(url.starts_with("data:image/png;base64,iVBORw0KGgo"));
        assert_eq!(resolver.data_url(name), Some(url));
        let rgba = render_icon_for_test(name);
        assert!(rgba.chunks_exact(4).any(|pixel| pixel[3] != 0));
    }

    fn render_icon_for_test(name: &str) -> Vec<u8> {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let mut icon = HICON::default();
        // SAFETY: valid NUL-terminated path and output slot; icon is released below.
        unsafe {
            assert_eq!(
                ExtractIconExW(PCWSTR(wide.as_ptr()), 0, Some(&mut icon), None, 1),
                1
            );
            let pixels = render(icon).expect("render");
            DestroyIcon(icon).expect("destroy");
            pixels
        }
    }
}
