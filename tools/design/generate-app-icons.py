#!/usr/bin/env python3
"""Offline Voice Ribbon app exports and optional previews. Requires Pillow.

python tools/design/generate-app-icons.py --previews
The approved transparent PNG in assets/branding is the raster master.
Use --reference PATH with --previews for the dark/light comparison sheet.
Tray previews are opt-in (--tray-previews) and compile the actual Rust renderer.
"""

import argparse
import io
from pathlib import Path
import struct
import subprocess
import tempfile

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[2]
ICONS = ROOT / "apps/macafon-ui/src-tauri/icons"
MASTER = ROOT / "assets/branding/macafon-voice-ribbon.png"
SIZES = (16, 20, 24, 32, 40, 48, 64, 96, 128, 256, 512, 1024)


def app_images():
    """Fit the complete artwork inside a square with consistent transparent padding."""
    with Image.open(MASTER) as source:
        artwork = source.convert("RGBA")
    bounds = artwork.getchannel("A").getbbox()
    if bounds is None:
        raise ValueError("The app icon master is fully transparent")
    artwork = artwork.crop(bounds)
    images = {}
    for size in SIZES:
        image = Image.new("RGBA", (size, size))
        fitted = artwork.copy()
        extent = max(1, round(size * 0.92))
        fitted.thumbnail((extent, extent), Image.Resampling.LANCZOS)
        image.alpha_composite(fitted, ((size - fitted.width) // 2, (size - fitted.height) // 2))
        images[size] = image
    return images


def ico(images, destination, sizes):
    """Store each exported size as a separate lossless RGBA frame."""
    frames = []
    for size in sizes:
        stream = io.BytesIO()
        images[size].save(stream, format="PNG")
        frames.append(stream.getvalue())
    offset = 6 + 16 * len(frames)
    result = bytearray(struct.pack("<HHH", 0, 1, len(frames)))
    for size, frame in zip(sizes, frames):
        result.extend(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(frame), offset))
        offset += len(frame)
    destination.write_bytes(result + b"".join(frames))


def tray_previews(destination, temporary):
    source = (ROOT / "apps/macafond/src/tray.rs").read_text()
    module = source[source.index("mod icon {"):source.index("/// No native tray implementation")]
    masks = str(ROOT / "apps/macafond/assets/logo-alpha.bin").replace("\\", "/")
    module = module.replace("../assets/logo-alpha.bin", masks)
    harness = '''
#[derive(Default)]
struct TrayState { muted: bool, bypass: bool, connected: bool }
''' + module + '''
fn main() {
    let out = std::env::args().nth(1).expect("output directory");
    for flags in 0..8 {
        let state = TrayState { muted: flags & 1 != 0, bypass: flags & 2 != 0, connected: flags & 4 == 0 };
        for image in icon::render(&state, icon::target(&state)) {
            std::fs::write(format!("{out}/{flags}-{}.argb", image.width), &image.data).expect("write preview");
            assert_eq!(image.width, image.height);
        }
    }
    let state = TrayState { muted: true, connected: true, ..Default::default() };
    for step in 0..=12 {
        for image in icon::render(&state, step as f32 / 12.0) {
            std::fs::write(format!("{out}/bend-{step}-{}.argb", image.width), &image.data).expect("write transition");
        }
    }
}
'''
    rust = temporary / "tray-preview.rs"
    rust.write_text(harness)
    binary = temporary / "tray-preview.exe"
    subprocess.run(["rustc", "--edition=2024", str(rust), "-o", str(binary)], check=True)
    subprocess.run([str(binary), str(temporary)], check=True)
    for theme, color in (("dark", "#121316"), ("light", "#f2f1ee")):
        sheet = Image.new("RGB", (660, 490), color)
        draw = ImageDraw.Draw(sheet)
        ink = "#f2f1ee" if theme == "dark" else "#121316"
        draw.text((20, 14), f"MACAFON / TRAY / {theme.upper()} / actual size + 2x", fill=ink)
        for flags in range(8):
            name = "+".join(n for bit, n in ((1, "muted"), (2, "bypass"), (4, "error")) if flags & bit) or "normal"
            row, col = divmod(flags, 2)
            x, y = 20 + col * 330, 48 + row * 104
            draw.text((x, y), name, fill=ink)
            for size, dx in ((16, 0), (32, 110)):
                raw = (temporary / f"{flags}-{size}.argb").read_bytes()
                rgba = bytes(v for i in range(0, len(raw), 4) for v in (*raw[i+1:i+4], raw[i]))
                image = Image.frombytes("RGBA", (size, size), rgba)
                frame = Image.new("RGBA", (size, size), color)
                frame.alpha_composite(image)
                frame.save(destination / f"2026-10-17-icon-tray-{name}-{size}-{theme}.png")
                sheet.paste(frame.convert("RGB"), (x + dx, y + 24))
                sheet.paste(frame.resize((size * 2, size * 2), Image.Resampling.NEAREST).convert("RGB"), (x + dx + 38, y + 24))
        sheet.save(destination / f"2026-10-17-icon-tray-{theme}.png")
    sheet = Image.new("RGB", (13 * 48, 72), "#121316")
    for step in range(13):
        raw = (temporary / f"bend-{step}-32.argb").read_bytes()
        rgba = bytes(v for i in range(0, len(raw), 4) for v in (*raw[i+1:i+4], raw[i]))
        sheet.paste(Image.frombytes("RGBA", (32, 32), rgba), (step * 48 + 8, 20), Image.frombytes("RGBA", (32, 32), rgba))
    sheet.save(destination / "2026-10-17-icon-tray-transition.png")


def app_comparison(images, reference, destination):
    """Show native-size exports next to the supplied reference, without network access."""
    with Image.open(reference) as source:
        reference_image = source.convert("RGBA")
    reference_image.thumbnail((256, 256), Image.Resampling.LANCZOS)
    sheet = Image.new("RGB", (1000, 704))
    draw = ImageDraw.Draw(sheet)
    for row, (theme, background, ink) in enumerate((
        ("DARK", "#121316", "#f2f1ee"),
        ("LIGHT", "#f2f1ee", "#121316"),
    )):
        top = row * 352
        draw.rectangle((0, top, 1000, top + 352), fill=background)
        draw.text((24, top + 16), f"MACAFON / APP ICON / {theme} / native pixel sizes", fill=ink)
        sheet.paste(reference_image, (24, top + 64), reference_image)
        draw.text((24, top + 44), "Owner reference", fill=ink)
        for size, x in ((256, 324), (48, 636), (32, 764), (16, 884)):
            draw.text((x, top + 44), f"{size} px", fill=ink)
            sheet.paste(images[size], (x, top + 64 + (256 - size) // 2), images[size])
    sheet.save(destination / "2026-10-17-icon-app-compare.png")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--previews", action="store_true")
    parser.add_argument("--reference", type=Path, help="Local reference image for comparison")
    parser.add_argument("--tray-previews", action="store_true")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="macafon-icons-") as directory:
        temporary = Path(directory)
        images = app_images()
        for name, size in (("icon", 512), ("32x32", 32), ("128x128", 128), ("128x128@2x", 256)):
            images[size].save(ICONS / f"{name}.png")
        ico(images, ICONS / "icon.ico", tuple(size for size in SIZES if size <= 256))
        images[1024].save(ICONS / "icon.icns", append_images=[images[s] for s in (16, 32, 64, 128, 256, 512)])
        public = ROOT / "apps/macafon-ui/public"
        images[32].save(public / "favicon.png")
        ico(images, public / "favicon.ico", (16, 32, 48))
        hicolor = ROOT / "packaging/icons/hicolor"
        for size in SIZES[:-1]:
            destination = hicolor / f"{size}x{size}/apps/macafon.png"
            destination.parent.mkdir(parents=True, exist_ok=True)
            images[size].save(destination)
        if args.previews or args.tray_previews:
            destination = ROOT / ".artifacts/screenshots"
            destination.mkdir(parents=True, exist_ok=True)
            if args.previews:
                for size in (256, 48, 32, 16):
                    images[size].save(destination / f"2026-10-19-voice-ribbon-{size}.png")
                if args.reference:
                    app_comparison(images, args.reference, destination)
            if args.tray_previews:
                tray_previews(destination, temporary)
    print("Exported Voice Ribbon app, favicon and Linux icons from the PNG master (offline).")


if __name__ == "__main__":
    main()
