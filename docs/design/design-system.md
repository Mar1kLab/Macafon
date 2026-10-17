# Macafon — design system

> Part of the [Macafon PRD](../PRD.md). Based on the visual language of [getartcraft.com/apps](https://getartcraft.com/apps)
> (tokens taken from the site's production CSS), adapted for an audio utility. **Dark theme only.**

## 1. Principles

1. **A tool, not a toy.** A "lab instrument" / HUD aesthetic: monospaced labels, thin lines,
   crosshair "ticks" in the corners, section numbering `01 / MICROPHONE`.
2. **Minimalism with an accent.** An almost monochrome palette; color appears only where it carries
   meaning: channel color, active module, signal level, warning.
3. **Creativity in the details.** An italic serif (Instrument Serif) for one word in a heading
   ("Your voice, *refined*."), "live" meters, ruler sliders with ticks, gentle micro-animations.
4. **Sharp corners.** `border-radius: 0` on UI components (as on ArtCraft). The approved logo is the exception: rounded contours and line caps. Separation by 1px lines, not shadows.
5. **Sound is visible.** Every module that changes the signal shows it visually (meter, gain reduction,
   EQ curve).

## 2. Color tokens (dark)

```css
:root {
  color-scheme: dark;
  /* Surfaces */
  --bg:            #121316;  /* window background */
  --bg-raised:     #101014;  /* module cards, panels */
  --bg-sunken:     #060607;  /* inputs, slider tracks, badges, meters */
  /* Text */
  --ink:           #f2f1ee;  /* main text */
  --ink-strong:    #ffffff;  /* headings, values */
  --muted:         #f2f1ee99;/* secondary text */
  --faint:         #f2f1ee61;/* captions, placeholders, inactive ticks */
  /* Lines */
  --line:          #f2f1ee26;/* card borders, dividers */
  --line-strong:   #f2f1ee66;/* crosshair ticks, focus, hover borders */
  /* Accent and states */
  --accent:        #2d81ff;  /* main accent (buttons, active toggles) */
  --accent-ink:    #74aaff;  /* accent text/icons on dark */
  --danger:        #ff6259;  /* clipping, mute, errors */
  --invert-bg:     #ffffff;  /* inverted button (primary CTA) */
  --invert-fg:     #0b0b0c;
  --scanline:      #ffffff06;/* background "scanline"/grid */
}
```

### 2.1 Module and channel colors

The `--app` / `--app-ink` palette from ArtCraft. `--app` — fills (channel bar, meter fill, EQ points),
`--app-ink` — text/icons on a dark background.

| Token | `--app` | `--app-ink` (dark) | Default use |
|---|---|---|---|
| blue | `#2f7bf5` | `#7fb0ff` | **Main** channel, Gain module |
| violet | `#8b5cf6` | `#b69cff` | **Voice** channel, Noise Suppression module |
| pink | `#e0368f` | `#ff7ab8` | **Music** channel, De-esser module |
| orange | `#e8573f` | `#ff8a70` | Limiter module, user channel |
| amber | `#f2a516` | `#ffc14d` | Compressor module, warnings (level −6…0 dBFS) |
| teal | `#12a58a` | `#3fd6b8` | EQ module, "signal OK" |
| lime | `#7bb51c` | `#a6e04a` | Expander/Gate module, HPF |

User channels pick their color from the same palette (7 colors).

### 2.2 Meter scale

- `−∞…−18 dBFS` — `--app-ink` of the current module/channel (dimmed, 60% opacity)
- `−18…−6 dBFS` — `--app-ink` 100%
- `−6…−1 dBFS` — amber `#ffc14d`
- `> −1 dBFS / clip` — `--danger`; the clip indicator "sticks" for 2 s

## 3. Typography

Fonts are bundled with the application (all under SIL OFL 1.1), never loaded from the network.

| Role | Font | Parameters |
|---|---|---|
| Display (screen titles, channel names) | **Archivo** | 600–700, `letter-spacing: -0.03em`, 24–40px |
| Body (descriptions, settings) | **Inter** | 400/500, 13–14px, `line-height: 1.5` |
| HUD labels, parameter values, badges | **Geist Mono** | 500, 11px, `uppercase`, `letter-spacing: 0.15em` (labels) / 0.1em (10px badges); values — 12–13px, tabular-nums |
| Accent word | **Instrument Serif** italic | One word in a screen/onboarding heading |

```css
.hud-label { font-family: var(--font-mono); font-size: .6875rem; font-weight: 500;
             letter-spacing: .15em; text-transform: uppercase; color: var(--muted); }
.badge     { font-family: var(--font-mono); font-size: 10px; letter-spacing: .1em;
             text-transform: uppercase; padding: 1px 6px; border: 1px solid var(--line);
             background: var(--bg-sunken); }
```

## 4. Grid, spacing, shapes

- Base unit `4px` (`--spacing: .25rem`), card padding 24px (`p-6`), 32px on a wide window.
- Minimum window size 960×640, default 1180×760. A 64px navigation sidebar (icons) with HUD captions on hover.
- `border-radius: 0` on all elements except the round EQ points; application chips are rectangular too (also 0, with an icon).
- 1px `--line` borders; hover — `--line-strong`; focus-visible — 1px `--accent` + outline offset 2px.
- **Ticks (crosshair):** 11×11px `--line-strong` crosshairs in the corners of the active card / selected area (like `.tick` on ArtCraft).
- **Window background:** a barely visible 24px grid of `--scanline` — the feel of an instrument's graph paper.

## 5. Motion

- Base animation: `150ms cubic-bezier(.4,0,.2,1)`; card appearance/hover: `300ms ease-out`.
- Application chip hover: `translateY(-4px)` + `drop-shadow(0 6px 14px rgba(0,0,0,.28))` (the icons on ArtCraft).
- Drag & drop: the lifted chip follows the cursor with a slight tilt (2°); the target channel is highlighted
  with a border in its `--app` color and corner ticks.
- Meters — 60 fps, 1 s peak hold, a smooth 20 dB/s fall.
- Respect `prefers-reduced-motion`: disable the tilt, translate and appearance animation.

## 6. Components

| Component | Description |
|---|---|
| **Module card** | A chain module card: number (`03`), HUD name, on/off toggle, main slider, gain reduction mini meter, `ADVANCED ▾` button. A disabled module — content at 40% opacity, the border stays |
| **Ruler slider** | A horizontal "ruler" slider: `--bg-sunken` track, ticks (every 10%, major every 50%), fill in the module color, monospaced value on the right. Double click — reset to default, wheel — step, Shift+wheel — fine step, clicking the value — keyboard entry |
| **Fader** | A vertical channel fader (the same ruler style), scale in dB/percent |
| **Meter** | Vertical/horizontal, segmented (2px segments with a 1px gap) — an "instrument" look |
| **EQ graph** | Canvas/WebGL: frequency grid (log), a 2px `--app-ink` curve of the total response, an 8% opacity fill under the curve, band points (drag: X — frequency, Y — gain, wheel — Q) |
| **App chip** | A rectangular chip: 20px application icon, name, a mini activity indicator (pulses while the application plays audio) |
| **Channel strip** | A channel column: a 3px colored bar on top, Archivo name, application chips, fader, meter, `M`, `EQ`, `FX` buttons |
| **Device picker** | A device dropdown; every microphone has a live mini level meter (you can see which microphone "hears") |
| **Transport (A/B)** | `● REC`, `▶ LOOP` buttons, an `A ⇄ B` toggle (large, inverted `--invert-bg` style) |
| **Toast** | Bottom right corner, `--line` border, HUD heading |

## 7. Icons

[Lucide](https://lucide.dev) (as on ArtCraft), 1.5px stroke, 16/20px. Application icons come from the
system (Linux: `application.icon-name` / `.desktop` file, Windows: the exe icon).

### 7.1 Macafon logo

The approved variant is **Retro**: a rounded microphone with five grille lines, mount screws, a double
stand and an oval base. The master is `assets/branding/macafon.svg`.
Export: `python3 tools/design/generate-logo.py` (Pillow, fontTools, system librsvg and Cairo; the UI
dependencies must be installed locally to read Archivo).
The ready SVG/PNG, ICO/ICNS, favicon, Linux hicolor and data for the UI/tray are stored in the repository;
the generator and its dependencies are not needed to build or run the application.

The working position is upright, `--ink-strong`; mute/bypass — red `--danger`, and the whole microphone
bends smoothly like rubber. Bypass is additionally marked with a pause sign, a connection error with a red
dot (BG-002). The transition takes 300 ms (`--dur-card`), does not repeat at rest and continues from the
current position on fast toggling. The UI respects `prefers-reduced-motion`. In the 16/22 px tray a
thicker, simplified outline with two grille lines is used. The white, black and red versions of the logo
have a transparent background. The launcher master is `apps/macafon-ui/src-tauri/icons/icon.svg`,
exported by `tools/design/generate-app-icons.py`: an asymmetric amber/orange speech cloud with a flat
bottom and tail, on the rounded dark tile (owner request, 2026-10-17). The filled light Retro microphone
has a dark grille and outline for contrast; 16..32 px exports simplify the grille and thicken the yoke.
The launcher uses the amber/orange palette above; its tile finish is independent of UI surfaces.

## 8. UI text tone

English, short, no exclamation marks. Labels are HUD-style nouns (`NOISE SUPPRESSION`, `THRESHOLD`),
hints are one line of plain language ("Cuts low rumble from desk bumps and fans").
