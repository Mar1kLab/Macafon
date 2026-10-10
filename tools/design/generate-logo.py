#!/usr/bin/env python3
"""Export Macafon's approved vector mark. Requires Pillow, fontTools and librsvg.

Run from any directory: python3 tools/design/generate-logo.py
Runtime consumers use checked-in exports and need none of these dependencies.
"""

import ctypes as C
import ctypes.util
import json
from pathlib import Path
from xml.etree import ElementTree as ET

from PIL import Image
from fontTools.pens.recordingPen import RecordingPen
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.svgLib.path import parse_path
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

ROOT = Path(__file__).resolve().parents[2]
BRAND = ROOT / "assets/branding"
NS = {"s": "http://www.w3.org/2000/svg"}
SOURCE = ET.parse(BRAND / "macafon.svg").getroot()
SHAPES = [(p.attrib["d"], float(p.attrib["stroke-width"])) for p in SOURCE.findall(".//s:path", NS)]
SIZES = (16, 22, 32, 48)
STEPS = 12
WHITE, BLACK, RED, BG = "#ffffff", "#000000", "#ff6259", "#121316"


def sample(source):
    pen = RecordingPen()
    parse_path(source, pen)
    contours, points = [], []
    current = start = None
    for op, args in pen.value:
        if op == "moveTo":
            if points:
                contours.append(points)
            current = start = args[0]
            points = [current]
        elif op == "lineTo":
            end = args[0]
            points.extend((current[0] * (1-t) + end[0]*t, current[1] * (1-t) + end[1]*t)
                          for t in (n/12 for n in range(1, 13)))
            current = end
        elif op == "curveTo":
            a, b, end = args
            for n in range(1, 25):
                t, u = n/24, 1-n/24
                points.append(tuple(u**3*current[k] + 3*u*u*t*a[k] + 3*u*t*t*b[k] + t**3*end[k]
                                    for k in range(2)))
            current = end
        elif op == "closePath":
            points.append(start)
            current = start
        elif op != "endPath":
            raise ValueError(op)
    if points:
        contours.append(points)
    return contours


CONTOURS = [(sample(d), w) for d, w in SHAPES]


def warp(x, y, amount):
    t = max(0, min(1, (113-y)/103))
    # Continuous rubber deformation shared with geometry.ts; no rigid rotation.
    from math import sin, pi
    return x + amount*(52*t**1.7 + 3*sin(pi*t)), y + amount*(32*t*t + (x-48)*.90*t)


def path_at(contours, amount):
    return "".join(" ".join(f'{"M" if i == 0 else "L"}{x:.3f},{y:.3f}'
                            for i, (x, y) in enumerate(warp(*p, amount) for p in contour))
                   for contour in contours)


def mark(color=WHITE, amount=0, small=False, bypass=False, error=False):
    paths = []
    for i, (contours, width) in enumerate(CONTOURS):
        # At panel sizes retain the silhouette, two grille bars and one stem.
        if small and i in (2, 4, 5):
            continue
        if small and i == 1:
            contours = [contours[0], contours[2]]
        if small and i == 6:
            contours = [sample("M48 93V106")[0]]
        d = path_at(contours, amount) if amount or small else SHAPES[i][0]
        paths.append(f'<path d="{d}" stroke-width="{max(width, 5.5) if small else width}"/>')
    body = f'<g fill="none" stroke="{color}" stroke-linecap="round" stroke-linejoin="round">'+"".join(paths)+"</g>"
    if bypass:
        body += f'<path d="M96 88V105M106 88V105" stroke="{color}" stroke-width="5" stroke-linecap="round"/>'
    if error:
        body += f'<circle cx="105" cy="22" r="8" fill="{RED}"/>'
    return body


def svg(body, size=128, viewbox="-8 0 128 128"):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" '
            f'viewBox="{viewbox}"><title>Macafon</title>{body}</svg>\n')


rsvg = C.CDLL(ctypes.util.find_library("rsvg-2"))
cairo = C.CDLL(ctypes.util.find_library("cairo"))
gobject = C.CDLL(ctypes.util.find_library("gobject-2.0"))
rsvg.rsvg_handle_new_from_data.argtypes = [C.c_char_p, C.c_size_t, C.POINTER(C.c_void_p)]
rsvg.rsvg_handle_new_from_data.restype = C.c_void_p
rsvg.rsvg_handle_render_cairo.argtypes = [C.c_void_p, C.c_void_p]
rsvg.rsvg_handle_render_cairo.restype = C.c_int
cairo.cairo_image_surface_create.argtypes = [C.c_int, C.c_int, C.c_int]
cairo.cairo_image_surface_create.restype = C.c_void_p
cairo.cairo_create.argtypes = [C.c_void_p]
cairo.cairo_create.restype = C.c_void_p
cairo.cairo_surface_write_to_png.argtypes = [C.c_void_p, C.c_char_p]
cairo.cairo_surface_write_to_png.restype = C.c_int
cairo.cairo_destroy.argtypes = [C.c_void_p]
cairo.cairo_surface_destroy.argtypes = [C.c_void_p]
gobject.g_object_unref.argtypes = [C.c_void_p]


def render(source, destination, size, height=None):
    data = source.encode()
    error = C.c_void_p()
    handle = rsvg.rsvg_handle_new_from_data(data, len(data), C.byref(error))
    if not handle:
        raise RuntimeError("Invalid SVG")
    surface = cairo.cairo_image_surface_create(0, size, height or size)
    context = cairo.cairo_create(surface)
    try:
        if not rsvg.rsvg_handle_render_cairo(handle, context):
            raise RuntimeError("SVG rendering failed")
        if cairo.cairo_surface_write_to_png(surface, str(destination).encode()) != 0:
            raise RuntimeError("PNG export failed")
    finally:
        cairo.cairo_destroy(context)
        cairo.cairo_surface_destroy(surface)
        gobject.g_object_unref(handle)
    # Tauri's compile-time icon loader requires RGBA even on an opaque background.
    with Image.open(destination) as image:
        rgba = image.convert("RGBA")
    rgba.save(destination)


for color_name, color in (("white", WHITE), ("black", BLACK), ("red", RED)):
    for state, amount in (("active", 0), ("inactive", 1)):
        name = f"macafon-{color_name}-{state}"
        (BRAND / f"{name}.svg").write_text(svg(mark(color, amount)))
        render(svg(mark(color, amount), 512), BRAND / f"{name}.png", 512)

# Outline the bundled Archivo font, so the lockup never needs a font download.
font_path = ROOT / "apps/macafon-ui/node_modules/@fontsource-variable/archivo/files/archivo-latin-wght-normal.woff2"
font = instantiateVariableFont(TTFont(font_path), {"wght": 650})
glyphs, cmap = font.getGlyphSet(), font.getBestCmap()
scale = 96 / font["head"].unitsPerEm
letters, advance = [], 0
for letter in "Macafon":
    glyph = glyphs[cmap[ord(letter)]]
    pen = SVGPathPen(glyphs)
    glyph.draw(pen)
    letters.append(f'<path d="{pen.getCommands()}" transform="translate({advance} 0)"/>')
    advance += glyph.width
lockup_width = round(256 + advance * scale + 24)
for color_name, color in (("white", WHITE), ("black", BLACK), ("red", RED)):
    source = (f'<svg xmlns="http://www.w3.org/2000/svg" width="{lockup_width}" height="256" '
              f'viewBox="0 0 {lockup_width} 256"><title>Macafon</title>'
              f'<g transform="translate(20 0) scale(2)">{mark(color)}</g>'
              f'<g fill="{color}" transform="translate(256 156) scale({scale} {-scale})">'
              + "".join(letters) + "</g></svg>\n")
    (BRAND / f"macafon-wordmark-{color_name}.svg").write_text(source)
    render(source, BRAND / f"macafon-wordmark-{color_name}.png", lockup_width, 256)

# UI morphs sampled contours; checked-in data is generated from the same master.
geometry = ROOT / "apps/macafon-ui/src/lib/brand/geometry.ts"
data = [{"width": w, "contours": [[[round(x, 4), round(y, 4)] for x, y in c] for c in contours]}
        for contours, w in CONTOURS]
geometry.write_text("// Generated by tools/design/generate-logo.py; edit assets/branding/macafon.svg.\n"
                    + "export const contours = " + json.dumps(data, separators=(",", ":")) + " as const;\n")

# Alpha-only tray frames avoid image decoding or geometry work in the daemon.
# Layout: frame-major, then sizes 16/22/32/48, one alpha byte per pixel.
mask = bytearray()
temporary = BRAND / ".frame.png"
for step in range(STEPS + 1):
    for size in SIZES:
        render(svg(mark(WHITE, step/STEPS, small=size <= 22), size), temporary, size)
        with Image.open(temporary) as image:
            mask.extend(image.getchannel("A").tobytes())
temporary.unlink()
(ROOT / "apps/macafond/assets/logo-alpha.bin").write_bytes(mask)

icons = ROOT / "apps/macafon-ui/src-tauri/icons"
def app_mark(size):
    return f'<rect width="128" height="128" fill="{BG}"/><g transform="translate(16 0)">{mark(small=size <= 32)}</g>'

app_body = app_mark(512)
(icons / "icon.svg").write_text(svg(app_body, 512, "0 0 128 128"))
for name, size in (("32x32", 32), ("128x128", 128), ("128x128@2x", 256), ("icon", 512)):
    render(svg(app_mark(size), size, "0 0 128 128"), icons / f"{name}.png", size)
small_icons = []
for size in (16, 24, 32):
    render(svg(app_mark(size), size, "0 0 128 128"), BRAND / ".frame.png", size)
    with Image.open(BRAND / ".frame.png") as image:
        small_icons.append(image.copy())
(BRAND / ".frame.png").unlink()
with Image.open(icons / "icon.png") as image:
    image.save(icons / "icon.ico", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)], append_images=small_icons)
    image.save(icons / "icon.icns")

public = ROOT / "apps/macafon-ui/public"
(public / "favicon.svg").write_text(svg(app_mark(32), 32, "0 0 128 128"))
with Image.open(icons / "icon.png") as image:
    image.save(public / "favicon.ico", sizes=[(16, 16), (32, 32), (48, 48)], append_images=small_icons)

hicolor = ROOT / "packaging/icons/hicolor"
scalable = hicolor / "scalable/apps"
scalable.mkdir(parents=True, exist_ok=True)
(scalable / "macafon.svg").write_text(svg(app_body, 128, "0 0 128 128"))
for size in (16, 24, 32, 48, 64, 128, 256, 512):
    destination = hicolor / f"{size}x{size}/apps"
    destination.mkdir(parents=True, exist_ok=True)
    render(svg(app_mark(size), size, "0 0 128 128"), destination / "macafon.png", size)
status = hicolor / "symbolic/status"
status.mkdir(parents=True, exist_ok=True)
for name, amount, bypass, error in (("normal", 0, False, False), ("muted", 1, False, False),
                                     ("bypass", 1, True, False), ("error", 1, False, True)):
    (status / f"macafon-{name}-symbolic.svg").write_text(svg(mark("currentColor", amount, True, bypass, error)))
print(f"Exported branding, platform icons, UI contours and {len(mask)} bytes of tray alpha frames.")
