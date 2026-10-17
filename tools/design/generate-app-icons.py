#!/usr/bin/env python3
"""Offline app exports and optional previews. Requires local Tauri CLI, Node, Pillow.

python tools/design/generate-app-icons.py --previews
The SVG is the master; small exports simplify the grille and use thicker strokes.
Use --reference PATH with --previews for the dark/light comparison sheet.
Tray previews are opt-in (--tray-previews) and compile the actual Rust renderer.
"""

import argparse
import io
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
from xml.etree import ElementTree as ET

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[2]
ICONS = ROOT / "apps/macafon-ui/src-tauri/icons"
CLI = ROOT / "apps/macafon-ui/node_modules/@tauri-apps/cli/tauri.js"
SIZES = (16, 24, 32, 48, 64, 128, 256, 512, 1024)
NS = "{http://www.w3.org/2000/svg}"
ET.register_namespace("", NS[1:-1])


def ico(images, destination, sizes):
    """Use explicitly rendered frames, including hinted small sizes."""
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
        images = {}
        for size in SIZES:
            tree = ET.parse(ICONS / "icon.svg")
            if size <= 32:
                for parent in tree.iter():
                    for child in list(parent):
                        if child.get("id") in ("detail", "screws"):
                            parent.remove(child)
                for element in tree.iter():
                    if element.get("id") == "grille":
                        element.set("d", "M36 32H60M36 53H60")
                    if element.get("id") == "stem":
                        element.set("d", "M48 93V106")
                    if element.get("id") == "microphone":
                        element.set("transform", "translate(31.36 21.84) scale(0.68)")
                        for stroke in element.iter():
                            if "stroke-width" in stroke.attrib:
                                stroke.set("stroke-width", "8" if size <= 24 else "6")
                    if element.get("id") == "tile":
                        element.set("transform", "translate(-2 -2) scale(1.03125)")
                if size == 16:
                    # A filled capsule survives one-pixel rasterization better than
                    # the outlined housing; retain the Retro grille, yoke and foot.
                    microphone = next(e for e in tree.iter() if e.get("id") == "microphone")
                    housing = next(e for e in microphone.iter() if e.get("fill") == "url(#housing)")
                    housing.set("fill", "url(#metal)")
                    grille = next(e for e in microphone.iter() if e.get("id") == "grille")
                    grille.set("stroke", "#6783ff")
                    grille.set("stroke-width", "8")
            source = temporary / f"{size}.svg"
            tree.write(source, encoding="unicode")
            subprocess.run(["node", str(CLI), "icon", str(source), "--output", str(temporary), "--png", str(size)], check=True, stdout=subprocess.DEVNULL)
            with Image.open(temporary / f"{size}x{size}.png") as image:
                images[size] = image.convert("RGBA")
        for name, size in (("icon", 512), ("32x32", 32), ("128x128", 128), ("128x128@2x", 256)):
            images[size].save(ICONS / f"{name}.png")
        ico(images, ICONS / "icon.ico", (16, 24, 32, 48, 64, 256))
        images[1024].save(ICONS / "icon.icns", append_images=[images[s] for s in (16, 32, 64, 128, 256, 512)])
        public = ROOT / "apps/macafon-ui/public"
        shutil.copyfile(ICONS / "icon.svg", public / "favicon.svg")
        ico(images, public / "favicon.ico", (16, 32, 48))
        hicolor = ROOT / "packaging/icons/hicolor"
        shutil.copyfile(ICONS / "icon.svg", hicolor / "scalable/apps/macafon.svg")
        for size in SIZES[:-1]:
            images[size].save(hicolor / f"{size}x{size}/apps/macafon.png")
        if args.previews or args.tray_previews:
            destination = ROOT / ".artifacts/screenshots"
            destination.mkdir(parents=True, exist_ok=True)
            if args.previews:
                for size in (256, 48, 32, 16):
                    images[size].save(destination / f"2026-10-17-icon-app-{size}.png")
                if args.reference:
                    app_comparison(images, args.reference, destination)
            if args.tray_previews:
                tray_previews(destination, temporary)
    print("Exported app, favicon and Linux icons from icon.svg (offline).")


if __name__ == "__main__":
    main()
