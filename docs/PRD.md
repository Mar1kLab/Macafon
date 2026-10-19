# Macafon — Product Requirements Document

| | |
|---|---|
| **Product** | Macafon — microphone processing and a per-channel audio mixer with virtual devices |
| **Document version** | 1.1 (draft) |
| **Date** | 2026-10-08 |
| **Status** | Under review |
| **Platforms** | Linux (PipeWire ≥ 1.0) — first; Windows 10/11 — second phase (virtual microphone on the free VB-CABLE, no driver of our own) |
| **License** | Open source, free: GPL-3.0-only for the application, MIT/Apache-2.0 for `macafon-dsp` |
| **Related documents** | [SteelSeries Sonar analysis](research/sonar-analysis.md) · [Design system](design/design-system.md) |

---

## Contents

1. [Summary](#1-summary)
2. [Problem](#2-problem)
3. [Goals and non-goals](#3-goals-and-non-goals)
4. [Users and scenarios](#4-users-and-scenarios)
5. [Release scope](#5-release-scope)
6. [Functional requirements — microphone](#6-functional-requirements--microphone)
7. [Functional requirements — output channels](#7-functional-requirements--output-channels)
8. [Background operation, tray, autostart, hotkeys, dictation](#8-background-operation-tray-autostart-hotkeys-dictation)
9. [UX and screens](#9-ux-and-screens)
10. [Non-functional requirements](#10-non-functional-requirements)
11. [Architecture](#11-architecture)
12. [Platform implementation](#12-platform-implementation)
13. [Configuration storage and presets](#13-configuration-storage-and-presets)
14. [Testing and quality](#14-testing-and-quality)
15. [Distribution](#15-distribution)
16. [Risks](#16-risks)
17. [Open questions](#17-open-questions)
18. [Success metrics](#18-success-metrics)
19. [Roadmap](#19-roadmap)
20. [Glossary](#20-glossary)

---

## 1. Summary

Macafon is a lightweight native Rust application that:

1. **Makes your voice clean.** It takes the physical microphone, runs it through a studio chain
   (Gain → High-pass → AI noise suppression → Downward expander → EQ → Compressor → De-esser → Limiter)
   and outputs the result into a system **virtual microphone `Macafon Microphone`**, which the user
   selects in Discord, OBS, games or as the system default.
2. **Splits audio into channels.** It creates virtual output devices (channels `Main`, `Voice`,
   `Music` + user channels), each with its own volume, EQ and effects. Applications appear in the
   interface automatically and are dragged between channels with drag & drop.
3. **Stays out of the way.** It runs in the background as a daemon with a tray icon, starts with the
   system, and the UI unloads from memory when the window is closed. Microphone processing latency is a
   few milliseconds.

The product repeats the strengths of SteelSeries Sonar (simplicity, D&D, presets) and fixes its
weaknesses (Windows only, a heavy host, latency, instability, a limited microphone chain).

## 2. Problem

- Cheap and mid-range microphones sound bad in a real room: hum, fan noise, keyboard, whistling "s",
  volume jumps. The built-in noise suppression of Discord/Zoom works differently in every application and
  cannot be tuned.
- Existing solutions are either tied to hardware/an ecosystem (Sonar, Wave Link, NVIDIA Broadcast), too
  complex (VoiceMeeter, EasyEffects), or paid/cloud-based (Krisp).
- **On Linux** there is no Sonar equivalent at all: there is the powerful EasyEffects and PipeWire graph
  editors, but no simple "mixer by category" with D&D and proper onboarding.
- The user needs one light program, not a bundle of 3–4 utilities.

## 3. Goals and non-goals

### 3.1 Product goals

| # | Goal | How we measure |
|---|---|---|
| G1 | A beginner sets up good microphone sound in ≤ 2 minutes | Onboarding completion time in usability tests (n ≥ 5) |
| G2 | Microphone processing adds practically no latency | Added latency ≤ 3 ms without AI noise suppression, ≤ 13 ms with RNNoise (see §10.1) |
| G3 | The application is unnoticeable in the background | Daemon CPU < 1% of a core without AI, RAM < 40 MB; UI closed → 0 MB |
| G4 | Audio never "breaks" | 0 xruns in a 1 h soak test; when Macafon crashes/exits, audio automatically returns to real devices |
| G5 | Application routing is simpler than in Sonar | D&D + auto-detection + remembered rules; 100% of the test applications (§14.4) move correctly |

### 3.2 Non-goals (out of scope for 1.0)

- macOS.
- ASIO, a professional multichannel routing matrix on the level of VoiceMeeter/Helvum.
- Hosting third-party plugins (VST3/CLAP/LV2) — roadmap.
- Spatial audio (HRTF/Spatial) — roadmap.
- A separate stream mix (Streamer mode) and ducking — roadmap.
- Several virtual microphones — roadmap (the architecture allows for it from the start).
- Live monitoring (hearing yourself in real time) and a spectrum analyzer — roadmap.
- Accounts, cloud, telemetry, ads — **never** (only opt-in crash reports in the future).
- UI localization — English only in 1.0 (strings go through an i18n layer from the start to avoid a rewrite).
- A light theme.

## 4. Users and scenarios

### 4.1 Personas

| Persona | Description | Main expectation |
|---|---|---|
| **Gamer** (primary) | Plays with friends in Discord, a headset/USB microphone, a noisy keyboard | "So they don't hear my keyboard and I sound loud and even; the game quieter, friends louder" |
| **Remote worker** | Calls in Zoom/Meet/Telegram, a laptop or USB microphone, a kitchen/open space | "So nobody hears the kid and the vacuum cleaner; turn it on and forget it" |
| **Beginner streamer/podcaster** | OBS, an inexpensive XLR/USB microphone | "To sound like the big streamers without learning sound engineering" |
| **Linux enthusiast** | Moved from Windows, misses Sonar/VoiceMeeter | "A proper GUI instead of editing PipeWire configs" |

### 4.2 Key scenarios (user stories)

- **US-1.** As a user, on first launch I choose my microphone from a list that shows which microphone
  "hears" me right now, so I don't have to guess by name.
- **US-2.** As a beginner, I go through auto-calibration (stay silent / say a phrase), and Macafon sets
  the gain, the gate threshold and the noise suppression strength itself.
- **US-3.** As a user, I record 10 seconds of my voice and switch A/B (before/after), moving sliders and
  hearing the result on a loop immediately.
- **US-4.** As a user, I move one "intensity" slider per module, and if I want to, I open "Advanced" and
  edit all the parameters.
- **US-5.** As a user, I select `Macafon Microphone` in Discord/OBS/as the system microphone and close the
  window — processing keeps running from the tray.
- **US-6.** As a user, I see all applications that play audio and drag Discord into the Voice channel and
  Spotify into Music; next time they land there automatically.
- **US-7.** As a user, I set Main to 30% and Voice to 50%, and enable noise suppression + compressor +
  limiter on Voice so that the other side sounds even and clean.
- **US-8.** As a user, I press a hotkey to mute the microphone and see it in the tray.
- **US-9.** As a user, I switch from headphones to speakers in Macafon once, and all channels move to the
  new device.
- **US-10.** As a user, if Macafon is closed or has crashed, audio in the system keeps working.

### 4.3 Reference output scenario (from the brief)

| Channel | Applications | Volume | Processing |
|---|---|---|---|
| 1 · **Main** | games, browser, everything else (the default channel) | 30% | EQ off |
| 2 · **Voice** | Discord, Telegram, TeamSpeak, Zoom | 50% | EQ on, noise suppression, compressor, limiter |
| 3 · **Music** | Apple Music / Cider, Spotify, Yandex Music | — | EQ on (music preset) |

## 5. Release scope

Priorities: **P0** — required for the release, **P1** — desirable, may move to a minor release,
**P2** — roadmap.

| Release | Platform | Content |
|---|---|---|
| **M0 · Spike** | Linux | A technical check: a PipeWire virtual source, latency measurement, RNNoise/DeepFilterNet in the RT thread, routing an application stream through metadata. No UI. |
| **M1 · MVP "Microphone"** | Linux | The whole microphone chain, onboarding with calibration, recording + A/B, presets, the virtual microphone, daemon + tray + autostart, basic meters |
| **M2 · 1.0 "Mixer"** | Linux | Output channels, auto-detection and D&D of applications, per-channel volume/EQ/FX, output device selection, hotkeys, Doctor diagnostics |
| **M3 · 1.x "Windows"** | Windows 10 22H2+/11 | A virtual microphone on the free VB-CABLE installed by the user (§12.2.1); the mixer as application volume groups without effects (§12.2.2); tray/autostart, an installer without administrator rights |
| **M4+** | both | See [Roadmap](#19-roadmap) |

---

## 6. Functional requirements — microphone

### 6.1 Device selection

| ID | Requirement | Priority |
|---|---|---|
| MIC-001 | A list of all physical input devices with a human-readable name (`node.description`) and a type icon (USB/built-in/headset/Bluetooth) | P0 |
| MIC-002 | Every device in the list has a live mini level meter so the user sees which microphone hears them. Meters are polled only while the list is open. The list updates on device connect/disconnect through a daemon event, without polling from the UI | P0 |
| MIC-003 | Macafon's own virtual devices and monitors (`*.monitor`) are excluded from the list | P0 |
| MIC-004 | Input channel selection for multichannel interfaces: `Left`, `Right`, `Mix (L+R)`. By default the channel with signal is auto-detected during calibration | P1 |
| MIC-005 | Hot-plug: when the selected microphone is disconnected, the virtual microphone keeps existing and outputs silence; a "Microphone disconnected" toast; on reconnect — an automatic return with no user action | P0 |
| MIC-006 | Fallback microphone: an optional second-priority one (for example, the built-in one) to switch to when the primary disconnects | P1 |
| MIC-007 | A settings profile is bound to the physical microphone (by a stable ID: `device.bus-path`/serial + name). Changing the microphone loads its profile | P1 |

### 6.2 Virtual microphone

| ID | Requirement | Priority |
|---|---|---|
| MIC-010 | The input device `Macafon Microphone` exists in the system for as long as the daemon runs (even with the physical microphone disconnected) | P0 |
| MIC-011 | Format: 48 kHz, float32, mono (a "stereo" option duplicates mono into 2 channels for applications that handle mono poorly) | P0 |
| MIC-012 | A "Set as system default" button (with confirmation) makes `Macafon Microphone` the default microphone; the original default is remembered and restored when Macafon is turned off / uninstalled. The daemon reports whether `Macafon Microphone` is the current default — the UI shows the state (a toggle), not two buttons | P0 |
| MIC-013 | Microphone mute (UI, tray, hotkey): digital silence is output, the physical microphone is not touched. A mute indicator in the tray | P0 |
| MIC-014 | Global Bypass: the whole chain is turned off (the signal passes with Gain = 0 dB), for instant comparison in a real call | P0 |
| MIC-015 | A hint in onboarding and on the microphone screen: "Turn off noise suppression / echo cancellation / auto gain in Discord, Zoom…" with a link to instructions, because double processing makes the sound worse | P1 |

### 6.3 Processing chain

The order is fixed (in 1.0) and follows vocal production practice:

```
Mic ─► [1 Gain] ─► [2 High-pass] ─► [3 Noise Suppression] ─► [4 Expander/Gate] ─► [5 EQ]
    ─► [6 Compressor] ─► [7 De-esser] ─► [8 Limiter] ─► [Output Gain] ─► Macafon Microphone
```

Why: an HPF before noise suppression removes hum that confuses the neural network; the expander after
noise suppression works on the already cleaned residue; EQ before the compressor so that the compressor
reacts to the shaped timbre; the de-esser after the compressor, because compression emphasizes
sibilants; the limiter last as clipping protection.

**General module requirements:**

| ID | Requirement | Priority |
|---|---|---|
| DSP-001 | Every module has an on/off toggle, **one main slider** (intensity/sensitivity, 0–100%) and an expandable **Advanced** block with all parameters | P0 |
| DSP-002 | The main slider is a "macro": it maps deterministically to a set of Advanced parameters (tables below). Changing any Advanced parameter by hand switches the module to `CUSTOM` mode (a badge); the main slider is locked with a "Reset to simple" button | P0 |
| DSP-003 | All parameter changes apply without clicks or artifacts: parameter smoothing (≥ 10 ms, linear/exponential), filter coefficients recomputed without discontinuities | P0 |
| DSP-004 | Dynamics modules (Expander, Compressor, De-esser, Limiter) show gain reduction in real time | P0 |
| DSP-005 | A disabled module is fully excluded from processing (0 CPU) and adds no latency | P0 |
| DSP-006 | The internal sample rate is 48 kHz. If a device runs at 44.1/96 kHz, PipeWire resamples (adapter), not our code | P0 |
| DSP-007 | All processing is 32-bit float, mono | P0 |
| DSP-008 | Double-clicking a slider resets it to the default; Undo/Redo (Ctrl+Z / Ctrl+Shift+Z) for parameter changes, depth ≥ 50 | P1 |
| DSP-009 | Reordering modules with drag & drop (Advanced mode) | P2 |

#### 6.3.1 Gain (input gain)

| Parameter | Range | Default | Note |
|---|---|---|---|
| Gain | −24 … +24 dB | 0 dB (or from calibration) | Main slider, 0.5 dB step |
| Auto (button) | — | — | Calibration: target speech level ≈ −18 dBFS RMS / peaks ≈ −6 dBFS |

Requirements: MIC-G1 (P0) — an input meter before and after Gain, an input clip indicator (clipping
**before** our processing cannot be fixed → the hint "Lower your mic level in system settings").
Windows automatic calibration lowers only the selected capture endpoint as needed to leave at
least 15 dB of raw speech-peak headroom; make-up gain stays digital and targets the same
−18 dBFS speech RMS. It never raises the user's endpoint level. The original endpoint level is
journaled for uninstall / explicit recovery; ordinary Quit preserves calibration
(owner request 2026-10-17).

#### 6.3.2 High-pass (low cut)

| Parameter | Range | Default |
|---|---|---|
| Frequency (main) | 20 … 300 Hz | 80 Hz |
| Slope | 12 / 24 dB/oct | 24 dB/oct |

Implementation: a cascade of Butterworth biquads (TPT/SVF form for stable modulation). Zero algorithmic latency.

#### 6.3.3 Noise Suppression (AI)

| Parameter | Range | Default | Note |
|---|---|---|---|
| Engine | `Lite` (RNNoise) / `Studio` (DeepFilterNet 3) | Lite | Switching without a restart, with a 50 ms crossfade |
| Intensity (main) | 0 … 100% | 70% | Lite: wet/dry mix + VAD threshold; Studio: `attenuation limit` 0 … 100 dB |
| Voice activity threshold (Lite, Advanced) | 0 … 100% | 50% | Extra attenuation when the voice probability is below the threshold |
| Post-filter (Studio, Advanced) | on/off | on | The DeepFilterNet post-filter for better suppression in pauses |

Requirements:
- NS-001 (P0) Lite = [`nnnoiseless`](https://github.com/jneem/nnnoiseless) (a Rust port of RNNoise), 10 ms frame @ 48 kHz.
- NS-002 (P0) Studio = DeepFilterNet 3 (the Rust crate `deep_filter` with `tract`), the model is embedded in the binary/resources, no network. If the CPU cannot keep up (the overload detector, §10.3) — an automatic fallback to Lite with a toast.
- NS-003 (P0) The UI honestly shows the added latency (`+10 ms` / `+~40 ms`) and load (`LOW CPU` / `MED CPU`) next to the engine choice.
- NS-004 (P1) Keyboard/mouse click suppression (like Impact NR in Sonar) — check the quality of RNNoise/DFN on clicks; if insufficient, a separate transient suppressor (P2).

#### 6.3.4 Downward Expander / Gate

| Parameter | Range | Default |
|---|---|---|
| Sensitivity (main) | 0 … 100% | 50% (→ threshold = noise floor + 6 dB after calibration) |
| Threshold | −80 … −10 dBFS | −50 dBFS |
| Ratio | 1:1.5 … 1:10, ∞ (= gate) | 1:3 |
| Range (max attenuation) | −80 … 0 dB | −30 dB |
| Attack | 0.1 … 50 ms | 2 ms |
| Hold | 0 … 500 ms | 80 ms |
| Release | 10 … 1000 ms | 150 ms |
| Hysteresis | 0 … 12 dB | 4 dB |

Macro: Sensitivity 0% → a high threshold (gently lets only loud speech through), 100% → the threshold at
the noise floor itself. Range and Ratio grow with "strictness". The detector is a 5 ms RMS envelope with a
100 Hz sidechain HPF (so hum does not open the gate). The graph shows a horizontal threshold line over the
input meter that can be dragged with the mouse.

#### 6.3.5 EQ (parametric)

| Parameter | Values |
|---|---|
| Bands | up to 10, 5 by default (Low shelf 120 Hz, Bell 250 Hz, Bell 1 kHz, Bell 4 kHz, High shelf 10 kHz — all 0 dB) |
| Types | Bell, Low shelf, High shelf, Low cut, High cut, Notch |
| Frequency | 20 Hz … 20 kHz |
| Gain | −18 … +18 dB |
| Q | 0.1 … 10 |
| Output gain | −12 … +12 dB |

- EQ-001 (P0) An interactive graph: the total frequency response curve, dragging points (X — frequency, Y — gain, wheel/Alt+drag — Q), double-clicking an empty area adds a band, right click — type/delete.
- EQ-002 (P0) Simple mode (the EQ's main "slider"): character presets — `Flat`, `Clarity`, `Warmth`, `Broadcast`, `Presence`, `Thin mic fix` + an `Amount 0–100%` slider that scales the gain of all preset bands.
- EQ-003 (P0) Biquads per the RBJ Audio EQ Cookbook formulas, an SVF implementation, zero algorithmic latency (a minimum-phase EQ).
- EQ-004 (P2) An overlay of the real signal's spectrum on the graph.

#### 6.3.6 Compressor

| Parameter | Range | Default |
|---|---|---|
| Amount (main) | 0 … 100% | 40% |
| Threshold | −60 … 0 dBFS | −20 dBFS |
| Ratio | 1:1 … 20:1 | 3:1 |
| Attack | 0.1 … 100 ms | 5 ms |
| Release | 10 … 1000 ms | 80 ms (+ an Auto release option) |
| Knee | 0 … 12 dB | 6 dB |
| Makeup | Auto / −12 … +24 dB | Auto |
| Detector | Peak / RMS | RMS |

Macro: Amount moves the threshold down (−10 → −35 dBFS) and the ratio up (2:1 → 6:1); auto makeup
compensates the loudness so that A/B compares character, not loudness. Feed-forward, no look-ahead.

#### 6.3.7 De-esser

| Parameter | Range | Default |
|---|---|---|
| Intensity (main) | 0 … 100% | 40% |
| Frequency | 3 … 12 kHz | 6.5 kHz |
| Threshold | −60 … 0 dBFS | automatic from Intensity |
| Max reduction | 0 … 18 dB | 8 dB |
| Mode | Split-band / Wideband | Split-band |
| Listen (Advanced) | on/off | off — listen to the sidechain band only, to find the "s" frequency |

Split-band is a dynamic high-shelf/bell at the frequency (no look-ahead); the detector is a band-pass sidechain.

#### 6.3.8 Limiter

| Parameter | Range | Default |
|---|---|---|
| Ceiling (main) | −12 … 0 dBFS | −1 dBFS |
| Release | 1 … 500 ms | 50 ms |
| Look-ahead | 0 / 1 / 2 / 5 ms | **0 ms** |

By default the limiter has no look-ahead (a fast attack + a soft clipper on the last 0.5 dB) so as not to
add latency. Look-ahead > 0 is an option for maximum quality (with the added latency shown explicitly).

#### 6.3.9 Output Gain

−24 … +12 dB, default 0. An output meter with short-term LUFS (P1) — the hint "target voice loudness for
Discord ≈ −18…−14 LUFS".

#### 6.3.10 Built-in microphone presets

| Preset | Essence |
|---|---|
| **Natural** (default) | HPF 80, Lite NS 60%, Expander 40%, EQ Flat, Comp 30%, De-ess 30%, Limiter −1 |
| **Discord Clear** | HPF 100, Lite NS 80%, Expander 60%, EQ Clarity 50%, Comp 50%, De-ess 40%, Limiter −1 |
| **Noisy Room** | HPF 120, Studio NS 90%, Expander 70%, EQ Presence 40%, Comp 50%, Limiter −1 |
| **Broadcast** | HPF 80, Lite NS 50%, Expander 40%, EQ Broadcast 70%, Comp 65%, De-ess 50%, Limiter −1 |
| **Warm Podcast** | HPF 70, Lite NS 50%, EQ Warmth 60%, Comp 55%, De-ess 45%, Limiter −1.5 |
| **Raw** | Everything off except Gain |

User presets: save / rename / duplicate / delete / export and import `.macafon-preset.json`. Built-in
presets are read-only (on change — "Save as new"). P0 — save/select, P1 — import/export.

### 6.4 Sound test: recording + A/B

The main tuning tool in 1.0 (live monitoring is on the roadmap).

| ID | Requirement | Priority |
|---|---|---|
| AB-001 | The `REC` button records the **raw** signal from the physical microphone (before the chain), 3–30 s long (10 s by default, stop with the button). A 3-2-1 countdown and the hint "Say something, then stay silent for 2 seconds" | P0 |
| AB-002 | `LOOP PLAY` plays the recording in a loop into the selected monitoring device (the current output by default, headphones recommended; a warning if the output is speakers) | P0 |
| AB-003 | During playback the recording is run **through the current chain in real time** by a separate processor instance — any slider change is heard immediately | P0 |
| AB-004 | An `A ⇄ B` switch (the `Space` key when the window has focus, `Tab`/`B` as an alternative): A = the raw signal, B = processed. Switching without a click (20 ms crossfade), a shared playback position | P0 |
| AB-005 | Loudness match (on by default): A is automatically matched in loudness to B so that "louder" does not seem "better" | P1 |
| AB-006 | Recording visualization: A and B waveforms overlaid, click to seek | P1 |
| AB-007 | The recording is kept only in memory; an "Export WAV" option (A and B) for a bug report / comparison | P2 |
| AB-008 | During recording/playback the main virtual microphone keeps working as usual | P0 |

### 6.5 Calibration (auto-setup)

| ID | Requirement | Priority |
|---|---|---|
| CAL-001 | Step 1: "Stay silent" — 4 s, measuring the noise floor (RMS, noise spectrum) | P0 |
| CAL-002 | Step 2: "Read this sentence" — a phrase on screen (~6 s), measuring the speech level and peaks | P0 |
| CAL-003 | From the results: Gain → speech at ≈ −18 dBFS RMS; Expander threshold = noise floor + 6 dB (not above speech − 12 dB); NS intensity by SNR (SNR > 40 dB → 40%, < 20 dB → 90% and a Studio suggestion); HPF by the presence of noise energy < 100 Hz | P0 |
| CAL-004 | A result screen: SNR before/after, "noise level: low/medium/high", exactly what was set (a list with module icons) and a "Listen A/B" button | P0 |
| CAL-005 | Windows: if raw speech peaks exceed −15 dBFS, automatically lower the selected capture endpoint in steps and check fresh capture; remeasure clipped speech. Journal the original endpoint level before changing it; restore on uninstall, `macafon restore-default` and `macafond cleanup`, but retain the calibrated level on Quit. Never raise the endpoint during calibration. Linux or unavailable/failed endpoint control: warn and ask the user to lower the system/interface level and retry. Digital Gain still targets ≈ −18 dBFS speech RMS (owner request 2026-10-17) | P0 |
| CAL-006 | Calibration can be run again from the microphone screen | P0 |

### 6.6 Meters

| ID | Requirement | Priority |
|---|---|---|
| MTR-001 | An input meter (after Gain) and an output meter (after Output gain): peak + RMS, peak hold, a clip indicator | P0 |
| MTR-002 | Gain reduction for Expander, Compressor, De-esser, Limiter | P0 |
| MTR-003 | An NS activity indicator (how much is being suppressed right now) | P1 |
| MTR-004 | Meters update at 60 Hz, only while the window is visible (otherwise the daemon sends no data) | P0 |

---

## 7. Functional requirements — output channels

### 7.1 Model

- **Channel** = a virtual output device (`Macafon Main`, `Macafon Voice`, `Macafon Music`, …) + its own
  processing chain + a fader. All channels are mixed into a shared **Master** and played into one real
  **output device**.
- Every application (audio stream) is assigned to exactly one channel. New applications land in the
  **default channel** (Main).

```
Firefox ─┐                      ┌──────────────────────────────┐
Steam game ─► Macafon Main  ─────►│ Main:  vol 30%  [EQ off]     │─┐
                                 └──────────────────────────────┘ │
Discord ───► Macafon Voice ──────►│ Voice: vol 50%  NS·EQ·Comp·Lim │─┼─► Master ─► Headphones
                                 └──────────────────────────────┘ │
Cider ─────► Macafon Music ──────►│ Music: vol 70%  EQ "Bass+"    │─┘
                                 └──────────────────────────────┘
```

### 7.2 Channels

| ID | Requirement | Priority |
|---|---|---|
| CH-001 | When the mixer is first enabled, 3 channels are created: **Main** (blue, default), **Voice** (violet), **Music** (pink) | P0 |
| CH-002 | The user can create a channel (up to 8 in total, on Windows too — WIN-CH-005), rename it, choose a color (a palette of 7) and an icon, delete it (except the default channel; the deleted channel's applications move to the default) | P0 |
| CH-003 | Any channel can be made the default channel | P1 |
| CH-004 | A volume fader 0–100% (shown in % and dB, a cubic volume curve as in PulseAudio/PipeWire), mute, solo (P1) | P0 |
| CH-005 | A channel meter (stereo, peak) | P0 |
| CH-006 | Master: fader, mute, meter, output device selection | P0 |
| CH-007 | Reordering channels by dragging | P1 |
| CH-008 | The volume of an individual application inside a channel (click a chip → a mini slider) | P1 |
| CH-009 | Turning the whole mixer on/off (if the user only needs the microphone). In M2 it is offered in onboarding on first launch, not enabled silently | P0 |
| CH-010 | Switching a single channel off (the strip switch or Channel settings; owner request, 2026-10-17): it loses its node, its apps play through the default channel and come back when it is on again; their rules stay. The default channel cannot be switched off, an off channel cannot become the default or receive apps | P1 |

### 7.3 Per-channel processing

The channel chain (stereo): `EQ → Noise Suppression → Compressor → Limiter → Volume`. All modules are off
by default, except those set by the channel template.

| ID | Requirement | Priority |
|---|---|---|
| CFX-001 | A 10-band parametric EQ (the same component as the microphone's) with presets | P0 |
| CFX-002 | EQ presets by category: Voice (`Voice Clarity`, `Bass Cut`, `Warm Voice`), Music (`Bass+`, `V-Shape`, `Vocal`, `Rock`, `Electronic`, `Classical`, `Podcast`), Game (`Footsteps`, `Immersive`, `Competitive`) | P0 |
| CFX-003 | Per-channel Noise Suppression (RNNoise, per L/R channel or a mono downmix for Voice) — to clean up the other side's voice | P0 |
| CFX-004 | Per-channel Compressor and Limiter (the same modules, stereo-linked) | P0 |
| CFX-005 | The default Voice channel template: EQ `Voice Clarity` on, NS 60% on, Compressor 40% on, Limiter −3 dBFS on, volume 50% | P0 |
| CFX-006 | The Main template: everything off, volume 30%. Music: EQ `Flat` on (the user picks a preset), volume 70% | P0 |
| CFX-007 | Channel bypass (all FX off with one click) | P1 |
| CFX-008 | Channel loudness normalization (evening out the loudness of different applications) | P2 |

### 7.4 Applications: auto-detection and drag & drop

| ID | Requirement | Priority |
|---|---|---|
| APP-001 | The daemon tracks all playback audio streams in real time (appearance/disappearance without polling) | P0 |
| APP-002 | A stream is grouped into an **application** by key: `application.process.binary` → Flatpak `pipewire.access.portal.app_id`/`application.id` → `application.name`. Several streams of one application (browser tabs, Discord voice + notifications) = one chip | P0 |
| APP-003 | An application chip: an icon (from `application.icon-name` or a `.desktop` file by binary/app_id), a readable name, a "playing now" indicator | P0 |
| APP-004 | Dragging a chip into another channel moves all of the application's streams **instantly and without interrupting audio** (without restarting the application) | P0 |
| APP-005 | The rule is remembered: next time the application starts, its streams land in the assigned channel right away (before audio plays in the wrong channel) | P0 |
| APP-006 | A built-in "known applications" database for auto-classification **on first appearance** (before a manual move): Discord, Vesktop, Telegram, TeamSpeak, Mumble, Zoom, Slack, Teams, Element → Voice; Spotify, Cider (Apple Music), Yandex Music, Tidal, Rhythmbox, Strawberry, Amberol, Deezer → Music; everything else → Main. Auto-classification can be turned off | P0 |
| APP-007 | Inactive (not playing now) applications with rules are shown dimmed in their channel; a "Show inactive" filter | P1 |
| APP-008 | Alternatives to D&D: a chip context menu "Move to → channel", the keyboard (chip focus + `1…8`) — accessibility | P0 |
| APP-009 | Applications that chose a specific output device themselves (not the default) are still taken over by a rule if the user dragged them; the "Don't manage this app" option excludes an application from Macafon | P1 |
| APP-010 | Browser tabs: in 1.0 a browser is one application. Splitting by tabs/sites — roadmap (P2) | P2 |
| APP-011 | System sounds (`media.role = event`/`notification`) — a separate "System sounds" pseudo-chip, in Main by default | P1 |

### 7.5 Output device

| ID | Requirement | Priority |
|---|---|---|
| OUT-001 | Choosing the real output device in the mixer header; all channels move at once | P0 |
| OUT-002 | A device priority list (for example, `USB DAC → Headset → Speakers`): when the active one disconnects — switch to the next available one; when a higher-priority one connects — switch back. No "reset to speakers at login" | P0 |
| OUT-003 | A "Follow system default" option — the output device = the system default (from before Macafon was enabled) | P1 |
| OUT-004 | Macafon channels do not appear in Macafon's own output device list | P0 |
| OUT-005 | `Macafon Main` becomes the system default sink when the mixer is enabled (otherwise new applications would bypass Macafon); the original default is remembered and restored when the mixer is disabled / on exit | P0 |

---

### 7.6 SoundDeck

Owner request, 2026-10-18: a soundboard with parity to the classic [Soundpad](https://www.leppsoft.com/soundpad/en/).
Sounds play into Macafon Microphone (the voice chat hears them) and into the user's headphones at
the same time. Windows only for now (`Capabilities::soundpad`); Linux reports it unavailable.

```
library folder -> decoder (48 kHz stereo, loudness/peak cache) -> SoundpadControl
  |-> limiter(voice * duck + normalized sound) -> CABLE Input
  \-> normalized monitor -> SoundDeck audio session -> headphones / chosen output
```

| ID | Requirement | Priority |
|---|---|---|
| SND-001 | A library folder (default `Music\Macafon Sounds`; any folder on any drive can be picked with the system folder dialog in Settings → SoundDeck or on the SoundDeck screen, and reset to the default — owner request, 2026-10-18; switching stops playback, settings and hotkeys of sounds stay by their relative paths): files in it and in its first-level subfolders (= categories) are the library; files copied in by hand and empty category folders are discovered by a background check every 3 s (plus filesystem scan time). Owner request, 2026-10-18: categories are physical first-level folders; app moves and category edits change those folders, and uploads land in the selected category using collision-safe names. External folder renames/moves preserve settings and both hotkeys when the disappeared/arrived file metadata match uniquely; ambiguous matches retain the old settings without guessing. Deeper folders, dot-prefixed entries, Windows hidden/system files, reparse points and unsupported files are ignored and left untouched. Category deletion moves managed sounds to the root; a folder containing ignored content remains until that content is removed in Explorer. Uploads stage in the ignored `.macafon-uploads` directory on the selected volume and finalize by rename. Open folder dispatches Windows shell activation off the control loop and acknowledges immediately; shell failures are logged. Rescan returns the current snapshot and publishes the completed background inventory asynchronously MP3, WAV, OGG, FLAC, M4A, AAC up to 200 MB and 10 minutes. The UI adds files by a file picker or by dropping them anywhere on the window (uploaded to the daemon in chunks; the SoundDeck screen opens), opens the folder, creates, renames and deletes categories, renames, moves and deletes sounds | P0 |
| SND-002 | Playing a sound replaces the current one (short fade, no clicks). It is mixed after the microphone chain into the cable, so mute/bypass of the voice do not affect it, and into a monitor stream on the default or a chosen output. Each target has an on/off switch and a 0-200 % volume; each sound has its own 0-200 % volume. At 100 %, sounds are loudness-normalized: microphone follows typical processed voice (-18 dBFS RMS fallback), headphones use -24 dBFS RMS. Normalization respects -1 dBFS sample-peak headroom; a linked limiter protects voice + sound after trims. The Windows monitor has its own movable **SoundDeck** app entry controlled by channel/app faders and mute, independently of the cable stream (owner request, 2026-10-18) | P0 |
| SND-003 | Transport: play/stop per sound, pause/resume, stop, repeat, seek on a progress ruler, play a random sound (of the visible list or a category); progress about 10 Hz | P0 |
| SND-004 | Every sound can have primary and secondary global hotkeys (`soundpad_play.<id>`, HK-009 rules); global actions stop, pause/resume and random. Owner request, 2026-10-18: automatic decoding ahead is bounded to the first four distinct hotkeyed sound IDs in sorted order, at most 8 MiB decoded each, one decode at a time and one attempt per file revision; other sounds decode on demand. Eviction does not trigger repeated idle preloads. Renaming or moving a sound keeps its settings and hotkeys | P0 |
| SND-005 | Library view: distinct rows, search, sort — newest first by default (file creation time), remembered when changed; oldest, name, plays, length, category — favorites, play counts, decode errors shown per sound; keyboard: arrows, Enter/Space play, F2 edit, Delete | P1 |
| SND-006 | The hotkey overlay (HK-007) has a soundpad bar with stop and a button that expands the sounds; every opening starts collapsed (owner request, 2026-10-18). Expanded: search (Enter plays the first match), a category filter and pads; the category, search and scroll position are remembered. Settings → Hotkey popup turns the soundpad bar on or off (on by default) | P1 |
| SND-007 | "Mute my voice while playing": while a sound plays into the microphone the voice fades out. The loudness reference observes processed voice before this duck, retaining its last value during silence/mute (owner request, 2026-10-18) | P1 |
| SND-008 | RT rules (AGENTS section 4.1): taps are allocation- and lock-free; decoded sounds are only freed on the control loop. Loudness/peak measurement and normalization coefficients are computed once at decode time; reference tracking, smoothed gains and zero-lookahead limiting allocate nothing (owner request, 2026-10-18) | P0 |

SoundDeck loudness policy (owner request, 2026-10-18): decode measures stereo RMS in 400 ms
blocks gated below -60 dBFS and sample peak, caching normalization coefficients with the PCM.
Normalization uses a maximum +40 dB gain and a -1 dBFS sample-peak ceiling before relative trims.
The microphone reference observes 400 ms processed-voice blocks before sound/duck, gated below
-50 dBFS, bounded to -36 to -12 dBFS RMS, with a 1/8 update per active block and 20 ms gain
smoothing. It starts at -18 dBFS RMS (CAL-005 speech target); silence retains the last reference.
This follows the actual chain output, including preset/AGC/compressor/output gain, rather than
assuming that raw calibration RMS is still the processed level. The headphones reference is
-24 dBFS RMS, 6 dB below the microphone fallback to leave comfortable monitor headroom; physical
listening volume still depends on the device/system level. Both taps use a stereo-linked -1 dBFS
peak limiter after trims, with immediate attack and 100 ms release; the microphone limiter includes
voice in its peak detector. It adds no lookahead latency. Non-finite/subnormal samples are sanitized.
These are gated RMS estimates, not claimed BS.1770 LUFS measurements.

No config migration: keep existing microphone/headphones/per-sound trims verbatim, including 1 %.
New-user defaults remain microphone 100 %, headphones 70 %. Automatically raising a saved low
headphones setting could surprise the listener; users may raise it after comparing the normalized
level. Windows opens the monitor in a dedicated, named WASAPI session, with a stable grouping GUID
and a separate mixer identity. Existing session volume journaling, external changes, meters and
channel assignment apply to it; no extra cable, driver, audio rerouting or Master multiplication.
This is a reversible use of ADR-0004's session mixer, not a replacement architecture.

SoundDeck scan/rescan probes files in the background. Permanent format/decode failures move files into
reserved `Unplayable/` (collision suffix ` (2)`); transient I/O failures never move files. This folder
is excluded from categories and normal sounds, and exposed separately as `problem_sounds` with a short
in-memory error. Individual deletion uses `soundpad.delete`; `soundpad.delete_problem_sounds` deletes
all problem files. Category creation/renaming cannot use the reserved name (SND-001).

## 8. Background operation, tray, autostart, hotkeys, dictation

### 8.1 Background and tray

| ID | Requirement | Priority |
|---|---|---|
| BG-001 | Closing the window (×) hides the UI to the tray, **the UI process ends** (the WebView unloads); audio processing continues in the daemon. On the first close **the daemon** shows a system notification (freedesktop Notifications) "Macafon keeps running in the tray": by then the UI process has ended and cannot show a toast itself | P0 |
| BG-002 | A tray icon with states: normal (a white upright microphone) / mic muted (red, with a cartoonish bend of the whole body) / bypass (the same bend + a pause sign) / error (bend + a `--danger` dot). A short smooth transition on state change; in the UI with `prefers-reduced-motion` — the final position right away. The Retro design is approved by the owner | P0 |
| BG-003 | The tray menu: `Open Macafon`, `Mute microphone` ✓, `Bypass processing` ✓, `Preset ▸` (list), `Output device ▸`, `Quit Macafon` | P0 |
| BG-004 | Left click on the icon opens/focuses the UI; middle click mutes the microphone (P1) | P0 |
| BG-005 | `Quit` stops the daemon: virtual devices disappear, system default devices are restored, application streams return to real devices | P0 |
| BG-006 | Single instance: launching `macafon` again focuses the existing window | P0 |
| BG-007 | GNOME without the AppIndicator extension: detect the missing SNI host and show instructions once (install "AppIndicator and KStatusNotifierItem Support") + the ability to control it by relaunching the application | P1 |

### 8.2 Autostart

| ID | Requirement | Priority |
|---|---|---|
| AS-001 | A "Start with system" option: in onboarding — an explicit checkbox, on by default, applied on `Done`. If onboarding is skipped, autostart is not changed silently — it is enabled in Settings | P0 |
| AS-002 | Linux: the daemon is a `systemd --user` service `macafond.service` (`Restart=on-failure`, `After=pipewire.service wireplumber.service`); a fallback for systems without a systemd user session — an XDG autostart `.desktop` | P0 |
| AS-003 | Start without a window (tray only); time until the virtual microphone is ready after PipeWire starts ≤ 1 s | P0 |
| AS-004 | Windows (M3): `HKCU\…\Run` for the daemon, no UAC; on by default after onboarding, because without the daemon `CABLE Output` is silent (WIN-MIC-005) | P0 (M3) |

### 8.3 Hotkeys

| ID | Requirement | Priority |
|---|---|---|
| HK-001 | Global actions: Toggle mic mute, Push-to-talk (hold = on), Push-to-mute, Toggle bypass, Next/Previous preset, Volume up/down for each channel and Master, channel/Master Mute | P0 |
| HK-002 | Assignment in Settings → Hotkeys: recording a shortcut by pressing it, conflict checks within Macafon | P0 |
| HK-003 | Wayland: through `xdg-desktop-portal` **GlobalShortcuts** (KDE Plasma, GNOME 48+, Hyprland). X11: through XGrab (the `global-hotkey` crate). Hotkeys are registered by **the daemon**, not the UI, so they work with the window closed | P0 |
| HK-004 | A fallback for environments without a portal: CLI `macafon ctl mic mute toggle` / `macafon ctl preset next` / `macafon ctl channel voice volume +5` — the user binds them in the DE settings; the UI shows a ready command to copy | P0 |
| HK-005 | A sound cue (a quiet "click") on mute/unmute — an option | P1 |
| HK-006 | An OSD on every hotkey action. Windows: Macafon's own popup on the right edge of the active monitor, no system notifications (owner decision, 2026-10-17) — a vertical volume slider in the channel's color with its icon and name, a compact card for mute/bypass/preset; spring animations, off when Windows animations are off. Settings → Hotkey popup: monitor (foreground/active or primary, shared with the hotkey overlay), style (Android-like vertical slider or Windows 11-like bar), edge (left/right or top/bottom) and an offset toward the center in percent of the screen. Linux: a replaceable desktop notification on preset/volume change | P1 |
| HK-007 | Quick panel (owner request, 2026-10-17): a small always-on-top window above the tray with Android/Windows 11-like tiles (microphone mute, processing bypass, mixer), the preset and pill sliders with mute for the Master and every channel that is on. Windows tray right-click opens this compact tray panel with Close, Open Macafon and Quit Macafon controls; the `quick_panel` hotkey or `macafon quick-panel` opens a centered horizontal overlay in the upper-middle of the foreground monitor. Both close on Esc, their trigger again or focus loss. Rounded corners are an owner exception to the design system, like the HK-006 popup | P1 |
| HK-008 | Default shortcuts (owner request, 2026-10-18): `LCtrl+LAlt+Z` quick panel, `RCtrl+PageUp`/`RCtrl+PageDown` Main volume, `RAlt+PageUp`/`RAlt+PageDown` Voice volume; retain `Ctrl+Alt+Q` mic mute and `Ctrl+Alt+R`/`Ctrl+Alt+F` Music volume. Apply once to an unoffered empty hotkey map or replace the exact full previous default map; preserve all customized, partial, secondary and cleared maps. Skip groups for absent channel IDs or unsafe AltGr typing keys. Windows enforces modifier sides; RAlt accepts AltGr synthetic LCtrl and rejects LCtrl+LAlt. Linux falls back to either-side Ctrl/Alt | P1 |
| HK-009 | Every action has primary and secondary shortcuts, independently recorded and cleared. Windows distinguishes left/right Ctrl and Alt; generic existing modifiers keep matching either side. Overlapping shortcuts are rejected. Releasing one of two held shortcuts leaves the hold action active until both are released | P0 |
| HK-010 | Settings provides a volume hotkey step of 1..20 percentage points (default 5), shared by channel and Master volume actions. Assigned shortcuts appear next to microphone, preset, channel and Master controls. Volume shortcuts repeat while held: one immediate step, a 400 ms initial delay, then one step every 100 ms; release or rebinding stops repetition. Toggle actions never repeat (owner request, 2026-10-17) | P0 |

### 8.4 Dictation (speech to text)

Owner request, 2026-10-19: Wispr Flow-like dictation that is **free and fully local**. Hold a
shortcut, speak, release: the text appears in the window that had focus. Required languages:
English, Ukrainian, Russian. Both engines below are implemented and the user picks one; no separate
engine benchmark gates the work. Model downloads from Hugging Face are authorized by the owner
(2026-10-19). Windows first (`Capabilities::dictation`); Linux reports it unavailable until §8.4.9.
Architecture decision: [ADR-0005](adr/0005-local-dictation-worker.md).

```
mic -> chain (tap point) -> [RT] atomic gate -> rtrb (2 s) -> [daemon STT thread] 48k -> 16k mono
                                                                    |  framed pipe
                                     macafon-stt (worker process, on demand, low priority)
                                       VAD trim -> engine (Whisper | Parakeet) -> text + language
                                                                    |
                         daemon -> language policy -> insert into the captured window -> popup
```

#### 8.4.1 Engines and models

| Engine | Runtime (license) | Model (Hugging Face, license) | Download | Languages | Notes |
|---|---|---|---|---|---|
| **Whisper** (default) | whisper.cpp through `whisper-rs` (MIT / Unlicense) | `ggerganov/whisper.cpp` · `ggml-large-v3-turbo-q5_0.bin` (MIT) — **Turbo**, default | ≈ 0.57 GB | ~100, EN/UK/RU strong | Best UK/RU accuracy; language can be forced (§8.4.4) |
| | | `ggerganov/whisper.cpp` · `ggml-small-q5_1.bin` (MIT) — **Small** | ≈ 0.19 GB | same | For weak PCs; noticeably weaker Ukrainian |
| | | `ggml-org/whisper-vad` · `ggml-silero-v5.1.2.bin` (MIT) — VAD | < 1 MB | — | Downloaded with any Whisper model |
| **Parakeet** | sherpa-onnx (Apache-2.0) + ONNX Runtime (MIT) | NVIDIA Parakeet TDT 0.6B v3, int8 sherpa-onnx export from its Hugging Face mirror (CC-BY-4.0) | ≈ 0.65 GB | 25 European incl. EN/UK/RU | Several times faster on CPU; detects the language itself, cannot be forced |
| | | Silero VAD v5 ONNX (MIT) | ≈ 2 MB | — | Downloaded with the Parakeet model |

Exact repository paths, file sizes and SHA-256 are pinned in a model manifest compiled into the
daemon (a Hugging Face commit, not `main`); refreshing the manifest is a code change. Attribution for
CC-BY-4.0 and the runtime licenses appears in Settings → About.

#### 8.4.2 Requirements

| ID | Requirement | Priority |
|---|---|---|
| STT-001 | Dictation is **off by default**. Off means: no tap in the RT path, no ring buffer, no worker process, no dictation hotkeys registered — zero CPU and RAM. Turning it on needs at least one downloaded model; without one, the toggle opens the model picker | P0 |
| STT-002 | Activation modes: **Hold** (push-to-talk; record while the shortcut is held) and **Hands-free** (press to start, press again to stop; a double tap of the Hold shortcut also starts it, as in Wispr Flow). `Esc` cancels in both modes and inserts nothing. A hold shortcut starts recording only after it is held for 250 ms, so brushing the keys or fast typing never starts it; the pre-roll (STT-005) keeps what was said meanwhile, and a release before 250 ms counts as a tap (owner report, 2026-10-19). Recordings under 300 ms are ignored. Maximum length 5 min; the popup warns at 4:30 and recording stops at 5:00 and is transcribed | P0 |
| STT-003 | Hotkey actions (HK-009 rules, two shortcuts each), Windows defaults following Wispr Flow (owner decision, 2026-10-19): `dictation_hold` push-to-talk `Ctrl+Win`; `dictation_toggle` hands-free `Ctrl+Win+Space`; `dictation_paste_last` re-inserts the last result into the focused window `Shift+Alt+Z`; `dictation_copy_last` copies the last result to the clipboard `Shift+Alt+X`; `dictation_hold_copy` push-to-talk that only copies the result to the clipboard, never inserting it `Ctrl+Win+Shift`; `Esc` cancels while recording (registered only then). Unassigned by default: `dictation_enable_toggle` (feature on/off) and `dictation_language_cycle` (Fixed installed layout languages in layout order → By layout). Modifier-only chords go through the existing `WH_KEYBOARD_LL` hook, either side of Ctrl/Shift/Alt/Win. The hook never swallows a modifier event (a swallowed Win release left Windows thinking Win was held for the whole system); instead it injects a tap of the unassigned virtual key 0xE8 when the chord fires, so releasing Win does not open the Start menu. The last result is kept in daemon memory even when history is off and cleared on Quit. Defaults are applied once like HK-008: never over a customized map, skipped on conflict | P0 |
| STT-004 | Audio tap (AGENTS §4.1): the RT thread checks one `AtomicBool`; while recording it copies the block into a preallocated `rtrb` (2 s at 48 kHz mono, allocated when the feature turns on, freed when it turns off). A daemon STT thread drains it, resamples 48 → 16 kHz mono (`rubato`, outside RT) and streams 16 kHz chunks to the worker during recording, so audio never waits for the release. Tap point: **Clean** (after noise suppression, before expander/compressor — default), **Raw** (after Gain) or **Processed** (chain output) | P0 |
| STT-005 | Pre-roll: while the feature is on, the tap keeps the last 300 ms in a small ring so the first syllable said together with the key press is not cut. It costs one copy of each block and nothing else. Can be turned off in Performance | P1 |
| STT-006 | "Mute my microphone while dictating" (default on): the virtual microphone output is muted for the recording and restored to its previous state afterwards (an existing mute stays). The voice chat does not hear the dictation. SoundDeck playback is not affected | P0 |
| STT-007 | Worker lifecycle: the key press starts recording **and** spawns `macafon-stt`, which loads the model in parallel (typical 0.5–2 s from SSD), hidden behind speech time. One worker at most. It is unloaded (process exits) after the **unload timeout**: Immediately / 1 / 3 (default) / 10 min / Never. The worker exits when its stdin closes and lives in a Job object with `KILL_ON_JOB_CLOSE`, so a daemon crash never leaves it running. A worker crash shows the error state in the popup and never affects audio | P0 |
| STT-008 | Low system impact: the worker runs at `BELOW_NORMAL_PRIORITY_CLASS` (EcoQoS only in game mode: on hybrid CPUs it parks the worker on efficiency cores, measured 13 s instead of 1 s for Parakeet), CPU by default, threads = max(2, physical cores / 2), adjustable 1..logical cores. **Game mode** (default on): when `SHQueryUserNotificationState` reports a full-screen D3D app or busy state at the key press, the job uses CPU even if GPU is chosen, keeps the thread limit, and the worker unloads right after the result regardless of the timeout | P0 |
| STT-009 | Two engines (§8.4.1), switchable in the UI; switching unloads the current worker. Engine-specific options are hidden for the other engine (language forcing and vocabulary are Whisper-only) | P0 |
| STT-010 | Language policy (§8.4.4): **By keyboard layout** (default), **Fixed** any Whisper-supported language, **Auto**. The layout is read from the target window at the key press. Parakeet supports only Auto; the UI says so | P0 |
| STT-011 | Silence and hallucination guard: VAD trims leading/trailing silence and long pauses before inference. If VAD finds under 200 ms of speech, nothing is inserted and the popup shows "No speech". Whisper runs with `no_speech` suppression; a whole-output match against a short list of known Whisper hallucinations ("Thank you for watching", "Продолжение следует…", "Дякую за перегляд" and similar) is dropped | P0 |
| STT-012 | Text insertion (§8.4.5): into the window captured at the key press; **Paste** (default, clipboard + `Ctrl+V`, clipboard restored) or **Type** (`SendInput` with `KEYEVENTF_UNICODE`). Options: trailing space (default on), what to do in full-screen games (Insert / Copy only (default) / Ignore) | P0 |
| STT-013 | Model manager (§8.4.6): list, download with progress, cancel, resume, SHA-256 verification, delete, disk-space check, import a local file that matches a manifest hash. Downloads start only from an explicit user action | P0 |
| STT-014 | Vocabulary: up to 50 user words/names (Whisper passes them as `initial_prompt`; Parakeet ignores them in 1.0). New profiles start with 40 English tech/product terms; existing lists stay. The prompt is tokenized and capped at 192 tokens, below Whisper's 224-token limit. Edited on the Dictation screen | P1 |
| STT-015 | History: the last 20 results with time, engine, language and duration; copy and re-insert from the Dictation screen and the overlay. Three modes (owner request, 2026-10-19): **Off**; **Session** — in daemon memory, cleared on Quit (default); **Saved** — also written to `%LOCALAPPDATA%\Macafon\dictation-history.json` (temp file + rename) and loaded at start. Clear history and switching away from Saved delete the file | P1 |
| STT-016 | Dictation popup (§8.4.7, §9.9.3): looks **Flow** (default) and **Analog**, position bottom-center (default) or top-center, vertical offset in percent, and its own monitor choice — follow the active window's monitor (default) or the primary only — independent of the hotkey popup's (owner request, 2026-10-19). Drawn natively by the daemon, so it works with the UI closed | P0 |
| STT-017 | Quick access: a **Dictation** tile in the tray quick panel and the hotkey overlay (HK-007), and `Dictation` ✓ in the tray menu (BG-003); all toggle STT-001. The tile shows the download progress when a model is downloading and is disabled with a hint when no model is present | P0 |
| STT-018 | Optional cues (default off): a quiet start/stop click played to the monitor output only, never into the virtual microphone | P1 |
| STT-019 | Privacy: audio and text never leave the machine; audio is never written to disk, text only in the opt-in Saved history (STT-015); logs contain only timings, engine, model, language and error codes | P0 |
| STT-020 | GPU acceleration (owner request, 2026-10-19): Whisper on the GPU through ggml's Vulkan backend (any vendor; `vulkan-1.dll` comes with the graphics driver), on by default with a GPU/CPU choice on the Dictation screen; a full-screen game forces the CPU (STT-008); a failed GPU load falls back to the CPU. Parakeet stays on the CPU. Measured: 13.4 s of speech in 0.29 s on an RTX 4070 vs 18.8 s on the CPU | P0 |
| STT-021 | CLI: `macafon stt transcribe <file.wav> [--engine] [--model] [--lang]` through the daemon, and `macafon ctl dictation on` / `off` / `toggle` for DE bindings and scripts | P1 |
| STT-022 | Models folder (owner request, 2026-10-19): default `%LOCALAPPDATA%\Macafon\models\stt`; any folder on any local drive can be picked with the system folder dialog and reset to the default. Changing it moves the downloaded models (copy → verify SHA-256 → delete the source; cancellable, progress shown; the loaded model is unloaded first; a failed move keeps the old folder in use). Network and removable drives are refused; free space is checked on the target drive. A folder on a rotational disk shows a hint that a cold start is slower (≈ 3–8 s instead of 0.5–2 s). Macafon writes only inside its own `<folder>\<engine>\<model>\` subfolders and never deletes anything else there | P0 |

#### 8.4.3 Process and protocol

- New crate `macafon-stt`: `trait SpeechEngine { fn transcribe(&mut self, pcm_16k: &[f32], opts: &TranscribeOptions) -> Result<Transcript> }`,
  implementations `WhisperEngine` and `ParakeetEngine` behind cargo features, VAD, the model manifest
  types. No dependency on the daemon or audio backends.
- New binary `apps/macafon-stt` (the worker), spawned by `macafond` with anonymous pipes. A small
  length-prefixed binary protocol: `Load {engine, model, vad, threads, device}`, `Audio {pcm_16k chunk}`,
  `Finish {language_policy_input}`, `Cancel`, `Unload` → `Loading`, `Ready`, `Result {text, language,
  lang_probs, timings}`, `Error {code}`. No sockets, no network in the worker.
- `Transcript.lang_probs` carries Whisper's probabilities restricted to {en, uk, ru}; Parakeet returns
  `None`.
- IPC (UI ↔ daemon): `dictation.set_config`, `dictation.models.{list,download,cancel,delete,import}`,
  `dictation.test.{start,stop}`, `dictation.history.{list,clear,reinsert}`; events `dictation.state`
  (`off | idle_unloaded | loading | ready | listening | transcribing | error`), `dictation.model_progress`,
  `dictation.result` (only to subscribed UI: test panel and history). The popup level comes from the tap
  at 60 Hz only while listening. Types are generated to TS from `macafon-ipc`.

#### 8.4.4 Language policy

Whisper confuses Ukrainian and Russian when it guesses freely, while the keyboard layout of the target
window usually says which of the two the user writes in. The layout is a strong hint, not a lock: the
common failure is an English layout left over from code or a password while the user dictates in
Russian; forcing `en` there would produce a transliteration.

1. At the key press: `GetForegroundWindow` → `GetWindowThreadProcessId` → `GetKeyboardLayout(thread)` →
   LANGID to ISO language code, validated against Whisper's supported languages. UWP: the thread of the
   child `CoreWindow` of `ApplicationFrameHost`. Console hosts report unreliable layouts → unknown.
2. **Fixed** → that language, no detection pass.
3. Otherwise Whisper detects the language (one extra encoder pass over the first 30 s window);
   probabilities are restricted to the installed keyboard-layout languages, or all supported languages when none are known, and renormalized. Other supported target layouts use their language directly; the following EN/UK/RU heuristic remains for those layouts.
4. `p_en > p_uk + p_ru` → **en** (Latin vs Cyrillic is reliable).
5. Cyrillic: layout uk/ru → the layout's language, unless the other one has `p ≥ 0.90`.
   Layout en/unknown → the last Cyrillic language used in this session unless the other has `p ≥ 0.90`,
   else the larger of `p_uk`, `p_ru`. The threshold is a constant tuned in D5.
6. **Auto** → the highest probability among installed layout languages (all supported languages when none are known).
7. The chosen language is passed to transcription, shown in the popup badge and remembered as the last
   Cyrillic one when applicable. Parakeet: always Auto, badge `AUTO`.

The daemon exposes installed layout languages (deduplicated in layout order) and all supported languages with English names. Fixed language defaults to the first installed supported layout, falling back to English. Language codes remain strings in configuration and IPC.

The layout is only read; Macafon never switches it. Insertion does not depend on it (`Ctrl+V`
uses the same virtual key in Latin and Cyrillic layouts; Unicode `SendInput` ignores layouts).

#### 8.4.5 Text insertion

- The target `HWND` is captured at the key press. On the result, if that window is still foreground →
  insert; otherwise → copy to the clipboard, popup "Copied — press Ctrl+V".
- **Paste:** read and keep the current clipboard (all formats that can be read), put the text with
  `ExcludeClipboardContentFromMonitorProcessing` and `CanIncludeInClipboardHistory = 0` so dictations
  do not land in Win+V history or clipboard sync, send `Ctrl+V`, restore the previous content after
  300 ms unless the clipboard changed meanwhile.
- **Type:** Unicode `SendInput` in batches; slower but leaves the clipboard alone.
- An elevated target (UIPI blocks input from a non-elevated process) → copy + popup hint. Macafon
  never elevates for this.
- Full-screen games (game mode detection): default **Copy only**, because injected input can matter to
  anti-cheats; the user may choose Insert.
- Text post-processing: trim, collapse whitespace, the trailing space option. No reformatting or
  rewriting of what was said.

#### 8.4.6 Model manager and downloads

- Storage: `<models folder>\<engine>\<model>\`, the folder from STT-022 (default
  `%LOCALAPPDATA%\Macafon\models\stt`). Downloads go to `*.part`, support HTTP
  `Range` resume, are verified against the manifest SHA-256 and finalized by rename; a mismatch deletes
  the file and shows an error.
- Source: only `https://huggingface.co/<repo>/resolve/<pinned commit>/<file>`, HTTPS with certificate
  validation, the system proxy honored. Redirects are followed only over HTTPS to Hugging Face's own
  hosts (`huggingface.co`, `hf.co` and their subdomains, e.g. its CDN), at most 5. No other host, no
  analytics, no auth token or cookies.
- Free-space check before starting (model size + 10 %). One download at a time, on a background
  thread at background I/O priority, outside the control loop; progress events at most 4 Hz.
- Delete is refused while the model is loaded (it is unloaded first). Import copies a local file and
  accepts it only if its hash matches a manifest entry.
- The installer never contains models; uninstall deletes downloaded models by default (in a custom
  folder only Macafon's own subfolders), with a
  checkbox to keep them (owner decision, 2026-10-19).

#### 8.4.7 Popup states

| State | Trigger | Flow look | Analog look |
|---|---|---|---|
| Loading | press, worker not ready yet, and listening | — (shown as Listening; loading is hidden behind speech) | the same |
| Listening | push-to-talk held / hands-free started | pill expands, 15 white bars react to the level | needle on the VU scale, a blinking `●` |
| Transcribing | release / hands-free stopped | bars collapse into a shimmering 3-dot row | needle falls back with VU ballistics, the scale dims, a scanning tick strip |
| Done | text inserted or copied | short check, then the pill shrinks away | `TYPED · 14 WORDS` or `COPIED` for 700 ms |
| No speech | STT-011 | a small shake, `No speech` | needle twitch, `NO SPEECH` |
| Error | worker crash, no model, download needed | pill tinted `--danger`, short text | `--danger` lamp, short text |
| Too long | 4:30 | the bars turn amber | the REC dot turns amber |

During push-to-talk the popup is click-through. In hands-free it accepts clicks: `■` stop and `✕` cancel.
The popup never takes focus. Springs and timing reuse the HK-006 popup engine and turn off with
Windows animations.

#### 8.4.8 Resource budget

| State | Extra RAM | CPU | GPU |
|---|---|---|---|
| Feature off | 0 | 0 | 0 |
| On, worker unloaded | ≈ 0.4 MB (rings) | one block copy for pre-roll (≪ 0.1 % of a core) | 0 |
| Worker loaded, idle | Whisper Turbo ≈ 0.8–1.2 GB; Small ≈ 0.3–0.5 GB; Parakeet ≈ 0.8–1.2 GB | ≈ 0 | 0 |
| Transcribing 10 s of speech | — | the thread limit for ≈ 0.3–1.5 s (Parakeet) / ≈ 1.5–4 s (Whisper Turbo) | 0 on CPU |

Numbers are estimates for the NFR-R1 reference CPU and are replaced by measurements in D1/D2
(NFR-R9…R10, NFR-L7).

#### 8.4.9 Linux (later)

Linux reports `dictation = false` in 1.0. Later: the same worker, text insertion through the
RemoteDesktop portal or `wtype`/`xdotool`, no per-window layout on Wayland (only Fixed and Auto).

---

## 9. UX and screens

The visual language is the [design system](design/design-system.md). The window is 1180×760 by default,
960×640 minimum.

### 9.1 Navigation map

```
┌──────┬──────────────────────────────────────────────────────────┐
│ ◉ Mic│  (Onboarding — a modal full-screen flow on first launch)  │
│ ≡ Mix│                                                           │
│      │                     screen content                         │
│ ⚙ Set│                                                           │
└──────┴──────────────────────────────────────────────────────────┘
 64px sidebar: Microphone · Mixer · Settings ; at the bottom — daemon status (a dot) and CPU %
```

On Windows the sidebar also has SoundDeck (§7.6) and Dictation (§9.9) when the daemon reports them.

### 9.2 Onboarding (first launch, M1)

1. **Welcome.** The heading "Your voice, *refined*." (Archivo + Instrument Serif italic), one line of
   description, a `Get started` button.
2. **Pick your mic.** A list of microphones with live meters ("Say something — the active mic will light
   up"). The device with the highest speech level is selected automatically.
3. **Calibrate.** Two steps (silence → a phrase) with a large circular progress indicator and a waveform.
4. **Result.** SNR before/after, the applied preset, `Listen A/B` (opens the A/B transport right in the
   step) and `Fine-tune` buttons. `Fine-tune` does not cut onboarding short: steps 5–6 are still shown
   (choosing the microphone in applications and autostart must not be lost), and after `Done` the
   Microphone screen opens instead of the last screen.
5. **Use it.** "Select **Macafon Microphone** in your apps", a `Make it the system default` toggle, hint
   cards for Discord / OBS / Zoom (with a reminder to turn off their own noise suppression).
6. **Run in background.** A `Start with system` checkbox (AS-001). A `Done` button. There is no separate
   "Start minimized to tray": on autostart only the daemon with the tray starts (AS-003), the UI window
   never opens.

M2 adds a **Mixer** step: "Split your sound into channels?" → a preview of three channels →
`Enable mixer` / `Skip`.

On Windows (M3), after Welcome comes an **Install VB-CABLE** step if the cable is not found (WIN-MIC-002);
the "Use it" step names the device `CABLE Output (VB-Audio Virtual Cable)` (WIN-MIC-003); the Mixer step
explains that channels on Windows control application volume without effects (WIN-CH-006).

Every step can be skipped; onboarding is available again from Settings.

### 9.3 Microphone screen

```
┌─ 01 / MICROPHONE ───────────────────────────────────────────────────────────────┐
│ [🎤 Blue Yeti ▾ ▮▮▮▯▯]   PRESET [Discord Clear ▾] [Save]   [⟳ CALIBRATE]  [BYPASS] [MUTE] │
├─────────────────────────────────────────────────────────────────────────────────┤
│ IN ▮▮▮▮▮▮▯▯▯ −21 dB                                       OUT ▮▮▮▮▮▮▮▯▯ −16 dB │
│ ┌ 01 GAIN ─────── [on]┐┌ 02 HIGH-PASS ─── [on]┐┌ 03 NOISE SUPPRESSION ─── [on]┐ │
│ │ |||||||█|||||  +4 dB││ ||█||||||||||  80 Hz ││ LITE ⇄ STUDIO   +10ms LOW CPU│ │
│ │            ADVANCED▾││            ADVANCED▾ ││ ||||||||||█||||   70%  ADV ▾ │ │
│ └─────────────────────┘└──────────────────────┘└──────────────────────────────┘ │
│ ┌ 04 EXPANDER ─── [on]┐┌ 05 EQ ──────────────────────────────────────── [on]┐ │
│ │ SENSITIVITY  55%    ││   ╭──╮        ●                                     │ │
│ │ GR ▮▮▯▯ −12 dB      ││ ──╯  ╰──●────╯ ╲___●____                  CLARITY ▾ │ │
│ └─────────────────────┘└────────────────────────────────────────────────────┘ │
│ ┌ 06 COMPRESSOR ─ [on]┐┌ 07 DE-ESSER ── [on]┐┌ 08 LIMITER ───────────── [on]┐ │
│ │ AMOUNT 40%  GR −4 dB││ INTENSITY 40% GR −3 ││ CEILING −1.0 dBFS   GR −0.5 │ │
│ └─────────────────────┘└─────────────────────┘└──────────────────────────────┘ │
├─────────────────────────────────────────────────────────────────────────────────┤
│ TEST  [● REC 10s]  [▶ LOOP]   ┃ A  RAW ⇄ B  PROCESSED ┃  ∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿  │
└─────────────────────────────────────────────────────────────────────────────────┘
```

- Cards follow the signal order (numbers 01–08 = position in the chain), with thin "wire" lines between them.
- EQ takes double width; clicking the EQ card expands the graph to full width.
- The TEST panel is pinned at the bottom — it is always at hand while tuning.
- Advanced expands inside the card (an accordion); the other cards move smoothly.
- Hovering any parameter shows a one-line hint in plain language.

### 9.4 Mixer screen (M2)

```
┌─ 02 / MIXER ───────────────────────────────────────── OUTPUT [🎧 Arctis Nova ▾] ─┐
│ ┌▔▔▔▔ blue ▔▔▔▔┐ ┌▔▔▔ violet ▔▔▔┐ ┌▔▔▔▔ pink ▔▔▔▔┐ ┌ ─ ─ ─ ─ ─ ┐ ┌── MASTER ──┐│
│ │ MAIN   DEFAULT│ │ VOICE        │ │ MUSIC        │ │  + NEW     │ │            ││
│ │ [🦊 Firefox  ]│ │ [💬 Discord ]│ │ [🎵 Cider   ]│ │  CHANNEL   │ │   ▮▮  │    ││
│ │ [🎮 Cyberpunk]│ │ [✈ Telegram]│ │              │ └ ─ ─ ─ ─ ─ ┘ │   ▮▮  █    ││
│ │ [🔔 System   ]│ │              │ │              │               │   ▮▮  │    ││
│ │      │  ▮▮    │ │     │  ▮▮    │ │     │  ▮▮    │               │  100%      ││
│ │      █  ▮▮    │ │     │  ▮▮    │ │     █  ▮▮    │               │  [M]       ││
│ │      │  ▮▮    │ │     █  ▮▮    │ │     │  ▮▮    │               └────────────┘│
│ │     30%       │ │    50%       │ │    70%       │                              │
│ │ [M] [EQ] [FX] │ │ [M] [EQ●][FX●]│ │ [M] [EQ●][FX]│                              │
│ └───────────────┘ └──────────────┘ └──────────────┘                              │
└──────────────────────────────────────────────────────────────────────────────────┘
```

- Application chips are dragged between columns; while dragging, the target column is highlighted with
  the channel color and corner ticks. Dropping outside a column cancels.
- `EQ` / `FX` open a drawer on the right with the EQ graph and the channel's modules (the same cards as
  the microphone's). A `●` dot on the button = the module is active.
- A new application appears in its channel with a short "flash" animation in the channel color.
- The microphone is not shown in the mixer (it has its own screen) — unlike Sonar, where Mic is the fifth channel.

### 9.5 Settings

| Section | Contents |
|---|---|
| General | Start with system, Close button behaviour (tray / quit), Check for updates (GitHub releases, opt-in), Re-run onboarding |
| Audio | Preferred quantum (64 / 128 / 256 / 512 frames; 128 = 2.7 ms by default), Output device priority list, Mic fallback device, Mono/Stereo virtual mic |
| SoundDeck | Library folder (SND-001) |
| Dictation | A link to the Dictation screen (§9.9), where all its settings live |
| Hotkeys | A list of actions, shortcut assignment, backend status (Portal / X11 / Unavailable → CLI commands) |
| Hotkey popup | Look, style, position, offset and monitor (HK-006), each shown as a picture of the screen with the popup where it appears |
| Quick controls overlay | What the overlay shows (§9.9.2 configurator), where side panels open and how the overlay is placed, as pictures; SoundDeck pads/list and its player bar (SND-006) |
| Doctor | Diagnostics (§9.6) |
| About | Version, third-party licenses (RNNoise, DeepFilterNet, whisper.cpp, sherpa-onnx, ONNX Runtime, the CC-BY-4.0 Parakeet attribution, fonts), GitHub links |

A section list on the left of Settings follows the scroll and jumps to a section (owner request, 2026-10-18).

### 9.6 Doctor (diagnostics)

A self-check with clear one-click fixes (like One-Click Fix in Sonar, but transparent):

- PipeWire/WirePlumber are running, versions ≥ the required ones.
- Macafon's virtual nodes exist and are linked to the right devices.
- `Macafon Microphone` is used by at least one application (a hint if not).
- Double processing detected (for example, EasyEffects or NoiseTorch also on this microphone).
- Xruns / DSP overloads in the last 10 minutes, the current quantum, DSP load.
- Hotkeys: whether the portal is available.
- A `Copy diagnostic report` button (text, without personal data, for a GitHub issue).

### 9.7 States and errors

| Situation | UI behavior |
|---|---|
| The daemon is not running | The UI starts it automatically; if that fails — a screen with the reason and a Retry button |
| PipeWire unavailable | A full-screen "PipeWire not found" message + a link to instructions |
| Microphone disconnected | The device card in the `DISCONNECTED` state, a toast, processing waits |
| DSP overload | A toast "Audio processing overloaded — switched Noise Suppression to Lite" |
| Input clipping | A red indicator on the IN meter + a hint |

### 9.8 Accessibility

- Full keyboard navigation (Tab order follows the chain, arrows move sliders, PageUp/Down — a large step).
- Text contrast ≥ 4.5:1 (`--ink` and `--muted` on `--bg` pass); meaning is never conveyed by color alone
  (mute = icon + color).
- ARIA roles for sliders (`role="slider"`, `aria-valuetext="−18 dB"`).
- UI scale 90–150% (Ctrl +/−). The layout never gets smaller than the minimum window (960×640 at 100 %): a smaller window caps the applied scale (owner request, 2026-10-18).

### 9.9 Dictation screen and popups

Requirements: §8.4. A sidebar item **Dictation** (Lucide `Speech`) after SoundDeck, shown when
`Capabilities::dictation` is true. All settings of the feature live on this screen; Settings has a
"Dictation" row linking to it. The mock daemon simulates models, downloads, worker states, levels and
transcripts (AGENTS §5).

#### 9.9.1 Layout

Owner request, 2026-10-19: the cards follow the order a user thinks in — the model, how it
understands you (language, vocabulary), the path from the microphone to the text and its history
(input, output, history), and last the resources it may use (performance, full width). The intro
heading shows only before the first model; the header carries the status, the main shortcuts and
the switch. The ASCII sketch below is the first version.

```
┌─ 04 / DICTATION ────────────────────── [● READY · WHISPER TURBO · 1.1 GB]  [DICTATION ▣ on] ─┐
│ Speak, it *types*.                                  HOLD  [Ctrl] + [Win]  ·  HANDS-FREE  [Ctrl]+[Win]+[Space] │
│ Local speech to text. Nothing leaves this computer.  PASTE LAST [Shift]+[Alt]+[Z] · Esc cancels │
├───────────────────────────────────────────────────────────────────────────────────────────────┤
│ ┌ 01 ENGINE ─────────────────────────────────────────┐ ┌ 02 LANGUAGE ───────────────────────┐ │
│ │ ⟨ WHISPER | PARAKEET ⟩                             │ │ ⟨ BY LAYOUT | FIXED | AUTO ⟩       │ │
│ │ ◉ TURBO   0.57 GB  QUALITY ▮▮▮▮▯ SPEED ▮▮▯▯▯ READY │ │ [EN] [UK] [RU]   (Fixed only)     │ │
│ │ ○ SMALL   0.19 GB  QUALITY ▮▮▯▯▯ SPEED ▮▮▮▮▯ [GET] │ │ Uses the keyboard layout of the   │ │
│ │   ||||||||||████████░░░░░░  48 %  12.4 MB/s  [✕]   │ │ window you dictate into.          │ │
│ └────────────────────────────────────────────────────┘ └────────────────────────────────────┘ │
│ ┌ 03 INPUT ─────────────────┐ ┌ 04 OUTPUT ─────────────────┐ ┌ 05 PERFORMANCE ──────────────┐ │
│ │ ⟨ RAW | CLEAN | PROCESSED⟩│ │ ⟨ PASTE | TYPE ⟩           │ │ UNLOAD ⟨NOW|1M|3M|10M|NEVER⟩ │ │
│ │ IN ▮▮▮▮▮▯▯▯ −22 dB        │ │ Trailing space      [on]   │ │ THREADS |||||█||||  6 / 12   │ │
│ │ Mute mic while       [on] │ │ In full-screen games       │ │ Game mode            [on]    │ │
│ │ dictating                 │ │ ⟨INSERT|COPY ONLY|IGNORE⟩  │ │ Pre-roll 300 ms      [on]    │ │
│ └───────────────────────────┘ └────────────────────────────┘ └──────────────────────────────┘ │
│ ┌ 06 POPUP ───────────────────────────────────┐ ┌ 07 VOCABULARY (Whisper) ─────────────────┐ │
│ │ [  FLOW  pic ] [ ANALOG pic ]  POSITION ⟨B|T⟩│ │ [Macafon] [Hrytsenko] [PipeWire] [+ add] │ │
│ └──────────────────────────────────────────────┘ └──────────────────────────────────────────┘ │
├───────────────────────────────────────────────────────────────────────────────────────────────┤
│ TRY IT  [ HOLD TO TALK ]   │ "Привіт, це перевірка диктовки."   1.2 s · TURBO · UK  [COPY]     │
│ HISTORY ▾ (20, in memory)  │ 12:41  Thanks, I'll send it tonight.        EN  0.9 s   [COPY][↵] │
└───────────────────────────────────────────────────────────────────────────────────────────────┘
```

- **Header.** The master toggle (STT-001) is the large `Toggle` in the header, like Bypass/Mute on the
  Microphone screen. Next to it, a status badge from `dictation.state`: `OFF`, `UNLOADED · 0 MB`,
  `LOADING`, `READY · <model> · <RAM>`, `LISTENING`, `TRANSCRIBING`, `ERROR` (`--danger`). The
  heading uses Archivo with one Instrument Serif italic word ("Speak, it *types*.").
- **Shortcut block.** Assigned shortcuts as keycaps (`.badge` style, 13px Geist Mono) for
  push-to-talk and hands-free, with paste last in the second line; a `MORE` link expands copy last
  and push-to-talk to clipboard. Clicking a keycap opens the HK-002 recorder for that action.
- **01 ENGINE.** A `Segmented` for the engine; below, one row per model: radio, name, size, two
  segmented 5-step meters (quality, speed — static manifest values, colored `--accent-ink`), and a
  state cell: `GET` (download), a ruler-style progress bar with percent, speed and `✕` cancel,
  `READY`, `ACTIVE` (loaded now) or `ERROR · RETRY`. A row menu has Delete and Import file. Below the
  rows: "Models are downloaded from Hugging Face", the total disk use and a storage line
  `STORAGE  C:\…\models\stt  [CHANGE] [RESET]` (STT-022) with the drive's free space; while models
  move, the line becomes a progress bar with cancel. A first-time empty state
  replaces the rows with one recommended card: "Whisper Turbo — best for EN / UK / RU — 0.57 GB —
  `Download`".
- **02 LANGUAGE.** Policy `Segmented`; in Fixed, chips of the user's keyboard-layout languages and a
  list of every other Whisper language (owner request, 2026-10-19: no fixed set of three). With Parakeet the
  card shows `AUTO ONLY` and a one-line hint, the controls are disabled (40 % content, border kept).
- **03 INPUT.** Tap point, a live horizontal `LevelMeter` of the chosen tap (subscription only while
  the screen is visible), the mute-while-dictating toggle.
- **04 OUTPUT.** Paste/Type, trailing space, the full-screen games policy.
- **05 PERFORMANCE.** Unload timeout, the threads `RulerSlider` (1..logical cores, default marked by a
  tick), game mode, pre-roll, CPU/GPU only when STT-020 ships. A small line under the card shows what
  the worker uses now: `UNLOADED · 0 MB` / `LOADED · 1.1 GB · CPU 6 THREADS`.
- **06 POPUP.** Two picture buttons (like the Hotkey popup section): a miniature screen with the
  Flow pill or the Analog meter where it will appear; the chosen one has corner ticks. Position
  bottom/top and an offset `RulerSlider`; a `Preview` button plays a 3 s demo of the real popup.
- **07 VOCABULARY.** Rectangular chips with `✕`, an input that adds a chip on Enter; hidden for
  Parakeet.
- **TRY IT** (pinned at the bottom like TEST on the Microphone screen). `HOLD TO TALK` records while
  the mouse button or Space is held, runs the whole pipeline but shows the text here instead of
  inserting it, with timing, model and language. **History** expands upward: rows with time, text
  (one line, ellipsis, full text on hover), language, duration, `COPY` and `↵` re-insert into the
  previously focused window.
- **First visit** (owner request, 2026-10-19): a four-step guide opens over the screen — Private
  (everything runs on this computer: no account, no cloud, works offline once the model is
  downloaded), Model, Talk (the hold shortcut, Esc) and Languages (the keyboard layouts found). Skip
  or finish marks it seen in the UI's local storage; a `?` button in the header opens it again.
- Keyboard and ARIA (§9.8): Tab order follows the cards; the hold button responds to Space down/up;
  model progress is `role="progressbar"` with `aria-valuetext="48 %, 12.4 MB per second"`.

#### 9.9.2 Overlay, quick panel and tray

- The hotkey overlay (owner request, 2026-10-19): the toggles are small 2×2 switches, filled when
  on and outlined when off; the main part is at least 816 px wide (a little wider than the first
  840 px window). Dictation and Sounds are switches too, each with a flag on the side its panel
  opens to: the Dictation switch turns dictation on and off, the Sounds switch and both flags open
  their panel (one at a time). A panel grows out of the overlay and folds back the same way (its
  width, or height when opening down, with a fade); the window follows it every frame, moved and
  sized in one call. The Dictation panel
  has the language mode (Layout / Fixed / Auto) with the fixed language, the engine, GPU/CPU and the
  history with copy and insert-again. The Sounds panel holds the pads; SoundDeck's transport and
  volumes dock along the bottom of the overlay with a slide-in.
- The tray quick panel and the hotkey overlay (HK-007) get a fourth tile **Dictation** (Lucide
  `Speech`) after Mixer. On = filled `--accent` tile like the existing ones; the subtitle shows the
  push-to-talk shortcut (`HOLD · Ctrl+Win`). While a model downloads, the tile shows a thin progress line
  at its bottom; with no model, it is dimmed and its subtitle reads `GET MODEL`, and a click opens the
  Dictation screen. A right click / long press on the tile opens the Dictation screen.
- On Windows the compact tray panel (HK-007) is the right-click menu; its Dictation tile is the tray
  toggle. No separate native menu is shown.
- Optional parts (configurator): an output device switcher (previous/next output, OUT-001) and a
  live level meter of the microphone output; the overlay window receives meters only while it shows
  the meter (the UI process keeps one daemon subscription for all its windows).
- The Sounds panel: search, a Favorites chip, one list of categories (All sounds and every category,
  so many categories do not crowd it) and the order; each pad has a star to add or remove the sound
  from favorites.

The hotkey overlay configurator (section 9.9.2, HK-007) persists `[ui.overlay]`: volumes, microphone,
processing and mixer switches, and preset picker default on; output-device switcher and microphone
meter default off. Side panels expand right (default), left or down. Placement is anchored (default,
main controls stay put) or centered (the whole overlay stays centered). Sounds and Dictation retain
their own `show_in_overlay` preferences. Updating overlay preferences never shows a sample popup.

#### 9.9.3 Dictation popup looks

Both looks are drawn by the daemon with Direct2D in the HK-006 popup thread (layered, topmost,
never focused), use only design tokens converted to constants, follow the HK-006 monitor policy and
turn their motion off with Windows animations. Default position: bottom-center, 48 DIPs above the
work area.

**Flow** (default; a Wispr Flow-like pill — an owner exception to `border-radius: 0`, like the HK-006
Classic popup and HK-007):

```
   idle → listening                        transcribing                 done
  ╭────────────────────────╮          ╭──────────────╮            ╭──────╮
  │  ▁▃▅▇▅▃▂▃▆▇▆▄▂▁▂        │          │   ·  •  ·     │            │  ✓   │
  ╰────────────────────────╯          ╰──────────────╯            ╰──────╯
     116×32 DIPs (150×32 hands-free)      88×28                       36×28 → fades
```

- Pill: `--invert-fg` fill at 92 % opacity, 1px `--line` hairline, a soft shadow (the HK-006
  Classic shadow). Appears from 40×8 with a spring (scale + fade, 220 ms).
- 15 vertical bars, 3 DIPs wide, 3 DIPs gap, `--ink-strong`, mirrored around the center line; heights
  follow 15 log-spaced band energies of the tap (computed in the daemon STT thread from 16 kHz audio,
  60 Hz) with 40 ms attack / 160 ms release, so quiet breathing barely moves them. The center bars are
  slightly taller (a gentle window) so the shape reads as a voice, not as a spectrum.
- Transcribing: bars collapse to three dots that shimmer left to right (opacity wave, 900 ms period).
- Done: a check morphs in, holds 400 ms, the pill shrinks to a point and fades. Copied: the check plus
  `Copied` text in Inter 12px.
- Hands-free: on hover the pill grows `✕` (cancel) on the left and `■` (stop) on the right.
- Owner request, 2026-10-19: no timer and no language in either look; both are smaller. The last
  30 seconds before the limit are shown by color only.

**Analog** (in the app's own style: a small, quiet VU meter, square corners, corner ticks; owner
request 2026-10-19: smaller and more minimal than the first version, livelier motion). 128×60 DIPs:
a thin arc with the red 0…+3 VU zone and five marks, no numbers; a white needle with VU ballistics
and a faint peak ghost that falls slowly; an amber lamp glow under the arc that breathes with the
voice; a blinking REC dot (no timer, no language), the state text after listening. Transcribing: the needle sways
and a light runs along the arc. The detailed description below is the first version and is kept
for its calibration and states:

```
  ┼──────────────────────────────────────────────┼
  │  -20  -10  -7  -5  -3  -1   0  +1  +2  +3      │
  │   '    '    '   '   '   '   ┃▌▌▌▌▌▌▌▌▌▌        │   ← red zone 0…+3 VU (--danger)
  │      ╲                                         │
  │        ╲             VU                        │   ← Instrument Serif italic
  │          ╲                                     │
  │ ▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔◉▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔ │   ← pivot under the shroud
  │ ● REC  00:07          WHISPER · TURBO      [UK] │   ← HUD labels, Geist Mono
  ┼──────────────────────────────────────────────┼
       280×132 DIPs, border-radius 0
```

- Frame: `--bg-raised`, 1px `--line-strong`, 11×11 corner ticks (design system §4), no shadow apart
  from the minimal one needed on bright desktops.
- Face: `--bg-sunken` with a faint amber "lamp" glow — a radial `--app-amber` gradient at 8 % opacity
  from the pivot, like a backlit meter. Scale: a thin arc in `--ink`, majors with Geist Mono 9px
  numbers, the 0…+3 VU segment as a thicker `--danger` arc. The word `VU` in Instrument Serif italic,
  `--muted`.
- **Calibration: 0 VU = −18 dBFS RMS**, the CAL-005 speech target. A calibrated voice swings around
  0 VU; the scale reads −20…+3 VU.
- Needle: 1.5 DIPs `--ink-strong` with a darker counterweight, true VU ballistics — a critically
  damped spring reaching 99 % in 300 ms with ≈ 1.5 % overshoot, driven by the tap RMS. A `PEAK` lamp
  (6×6 square, `--danger`) lights for 1 s when the tap exceeds −1 dBFS.
- Shroud: a `--bg-raised` bar covering the pivot, carrying the HUD line — `● REC` (the dot blinks
  `--danger` at 1 Hz) and the `mm:ss` timer, engine and model, a language `.badge`.
- Listening: the face glow fades in (150 ms), the needle wakes from the rest pin at −20.
- Transcribing: the needle falls back to the pin with its ballistics, the glow dims to 3 %, the HUD
  reads `TRANSCRIBING` and a strip of 2px ticks under the scale scans left to right (a ruler-slider
  echo).
- Done: `TYPED · 14 WORDS` / `COPIED` in the HUD for 700 ms, then the frame slides 12 DIPs down and
  fades (300 ms).
- No speech: a needle twitch to −15 and back, HUD `NO SPEECH`. Error: the `PEAK` lamp stays on and the
  HUD shows the short error. Too long: the timer turns `--app-amber-ink` at 4:30.
- Hands-free: `■ STOP` and `✕` as HUD buttons in the shroud (`.btn` ghost style), clickable.
- Reduced motion: no glow fade or slide; the needle still moves (it is the content), but without
  overshoot.

---

## 10. Non-functional requirements

### 10.1 Latency

**"Zero latency" is physically impossible**, but the goal is that Macafon adds nothing beyond one audio
graph cycle and the algorithmic latency of the selected modules.

| Component | Added latency |
|---|---|
| Buffering (processing in the same PipeWire graph cycle, quantum 128 @ 48 kHz) | 0 extra cycles; the cycle itself, 2.67 ms, is shared by any PipeWire chain |
| Gain, HPF, Expander, EQ, Compressor, De-esser, Limiter (0 ms look-ahead) | **0 ms** (sample by sample, minimum-phase) |
| Noise Suppression Lite (RNNoise) | **10 ms** (a 480-sample frame) |
| Noise Suppression Studio (DeepFilterNet 3) | **≈ 40 ms** (frame + model look-ahead); refined in M0 |
| Limiter look-ahead (option) | 1 / 2 / 5 ms |

**Requirements:**

| ID | Requirement | Target |
|---|---|---|
| NFR-L1 | Added latency of the virtual microphone without NS (beyond connecting an application directly to the physical microphone) | ≤ 3 ms at quantum 128 |
| NFR-L2 | The same with NS Lite | ≤ 13 ms |
| NFR-L3 | Output channels without NS | ≤ 1 quantum (≤ 3 ms at 128) |
| NFR-L4 | The latency of each chain is reported to the PipeWire graph (the `latency` param) so applications can compensate for it | — |
| NFR-L5 | The UI shows the actual total chain latency (`LATENCY 10.0 ms` in the screen header) | — |
| NFR-L6 | Windows: virtual microphone latency without NS (WASAPI capture + chain + VB-CABLE buffer + output) | ≤ 20 ms with the recommended cable buffer; refined in M3.0 |
| NFR-L7 | Dictation: key release → text inserted, 10 s of speech, warm worker, default threads on the NFR-R1 reference CPU | Parakeet ≤ 1.5 s, Whisper Turbo ≤ 4 s, Whisper Small ≤ 1.5 s; a cold worker adds only the part of the model load that outlasts the speech. Refined by measurement in D1/D2 |

Measurement method — §14.3.

### 10.2 Resources

| ID | Metric | Target |
|---|---|---|
| NFR-R1 | Daemon CPU: microphone, chain without NS, 3 channels without FX | < 1% of one core (reference: Ryzen 5 3600 / i5-10400) |
| NFR-R2 | CPU: + NS Lite on the microphone and on the Voice channel | < 3% |
| NFR-R3 | CPU: + NS Studio on the microphone | < 10% |
| NFR-R4 | Daemon RAM | < 40 MB (+ ~15 MB with the DeepFilterNet model loaded), excluding the soundpad decoded cache (up to 512 MiB) and active/in-flight decode buffers; automatic hotkey preloads target at most 32 MiB (owner request, 2026-10-18) |
| NFR-R5 | UI RAM (window open) | < 200 MB; window closed — the UI process has ended. **Windows deviation (owner decision, 2026-10-17):** the daemon keeps the UI process warm in the background with the hidden quick panel (~190 MB private, mostly WebView2), so the tray panel and the hotkey overlay open in ~15–40 ms instead of ~370 ms; it ends with the daemon |
| NFR-R6 | Install size | < 30 MB (without the system WebView runtime). **Deviation (owner decision, 2026-10-19):** the dictation runtimes (§8.4) add their measured size, estimated 15–25 MB; models are never in the installer |
| NFR-R7 | Zero activity when idle: meters/spectrum are neither computed nor sent while the UI is closed; owner request, 2026-10-18: unchanged Windows sound libraries poll a native change-notification handle every 3 s, without directory walks, header probes, decodes or library events | Background polling falls back to scans if notifications are unavailable; inventory and header I/O stay off the control/audio loop |
| NFR-R9 | Dictation idle cost (§8.4) | Feature off: 0 extra RAM/CPU, no worker. On with the worker unloaded: ≤ 1 MB extra daemon RAM and ≤ 0.1 % of a core. A full-screen game with dictation on but unused shows no measurable FPS change |
| NFR-R10 | Dictation worker | Loaded RAM ≤ 1.3 GB (Whisper Turbo, Parakeet), ≤ 0.6 GB (Whisper Small); exits within 1 s of the unload timeout; never above `BELOW_NORMAL` priority |
| NFR-R8 | Large sound libraries (owner request, 2026-10-18) | Startup inventories and duration probes run on one bounded worker (background CPU/I/O priority on Windows); apply inventories in linear time on the control loop. Automatic full-library events retain the protocol shape, coalesce to at most 1 Hz, and serialize on a bounded event worker; play counts do not rebind hotkeys or repeat preloads. Completed uploads acknowledge without a full-library response and publish through the coalesced event. Playback progress remains about 10 Hz |

### 10.3 Reliability and real-time safety

| ID | Requirement |
|---|---|
| NFR-S1 | In the audio thread: no allocations, mutexes, system calls, logging or IPC. Checked with `assert_no_alloc` in debug/test builds |
| NFR-S2 | Parameters from the UI → the audio thread through lock-free structures (atomics / the `rtrb` SPSC ring buffer); a preset change prepares the new state outside the RT thread and swaps a pointer atomically |
| NFR-S3 | Overload detector: if processing time > 80% of the cycle budget for 3 cycles in a row — degrade (Studio → Lite NS), an event in the UI |
| NFR-S4 | Protection against NaN/Inf/denormals: flush-to-zero (FTZ/DAZ) in the RT thread, a sanitizer at the output of every chain (NaN → 0 and a module state reset) |
| NFR-S5 | Soak test: 1 h without xruns caused by Macafon at quantum 128 on the reference machine |
| NFR-S6 | Daemon crash → systemd restarts it within ≤ 2 s; in between, application streams automatically move to real devices (PipeWire/WirePlumber fallback) |
| NFR-S7 | A UI crash never affects audio |
| NFR-S8 | Recovery after suspend/resume, a PipeWire restart, device changes — without user involvement |
| NFR-S9 | The daemon runs without root; RT priority comes through PipeWire mechanisms (RTKit / `rlimits`) for the data loop |

### 10.4 Privacy and security

- No telemetry and no network requests, except the opt-in update check (GitHub Releases API) and
  user-initiated dictation model downloads from pinned Hugging Face URLs (§8.4.6, owner authorization
  2026-10-19).
- A/B recordings are kept in memory only and deleted when the UI closes.
- The daemon's IPC socket is accessible only to the current user (`$XDG_RUNTIME_DIR/macafon/macafond.sock`, permissions 0600).
- Noise suppression models ship in the package; dictation models are downloaded on request. Both are
  verified by SHA-256.
- Dictation audio and text stay in memory and never reach disk or logs (STT-019).

### 10.5 Compatibility (Linux)

| Component | Minimum |
|---|---|
| PipeWire | 1.0 (test matrix: 1.0.x, 1.2.x, latest) |
| WirePlumber | 0.5 |
| Session | Wayland and X11 |
| DE (tested) | KDE Plasma 6, GNOME 46+, Hyprland, Cinnamon |
| Distributions (tested) | Ubuntu 24.04 / 26.04, Fedora 42+, Arch, Debian 13, Linux Mint 22 |
| Architectures | x86_64 (P0), aarch64 (P1) |
| WebView | WebKitGTK 4.1 (a Tauri dependency) |

---

## 11. Architecture

### 11.1 Processes

```
┌───────────────────────────────┐        IPC (Unix socket / named pipe)       ┌─────────────────────────┐
│ macafond  (daemon, always on)  │◄───────────────────────────────────────────►│ macafon  (UI, on demand)  │
│                                │  a JSON-RPC-like protocol:                   │ Tauri v2 + Svelte 5     │
│  ┌──────────────┐  ┌─────────┐ │  · commands (set_param, move_app, …)        │ WebKitGTK / WebView2    │
│  │ Control loop │  │ Tray    │ │  · events (devices_changed, app_added, …)   │ Exits when closed       │
│  │ (tokio)      │  │ (SNI)   │ │  · a 60 Hz meter stream (subscription only)  └─────────────────────────┘
│  └──────┬───────┘  └─────────┘ │                                               ┌─────────────────────────┐
│         │ lock-free            │◄──────────────────────────────────────────────│ macafon ctl (CLI)         │
│  ┌──────▼─────────────────────┐ │                                               │ DE hotkeys, scripts     │
│  │ Audio engine (RT data loop)│ │                                               └─────────────────────────┘
│  │ mic chain · channel chains │ │
│  │ A/B player                 │ │
│  └──────┬─────────────────────┘ │
│  ┌──────▼──────┐ Hotkeys        │
│  │ Backend:    │ (portal/X11)   │
│  │ PipeWire    │                │
│  │ (WASAPI M3) │                │
│  └─────────────┘                │
└─────────────────────────────────┘
```

**Why two processes:** the daemon must be minimal and unkillable (audio matters more than the UI); the
WebView weighs 100+ MB and is not needed 99% of the time. The tray lives in the daemon (on Linux — a pure
D-Bus StatusNotifierItem through the `ksni` crate, without GTK), so closing the UI does not remove the icon.

### 11.2 Repository layout (Cargo workspace)

```
Macafon/
├─ crates/
│  ├─ macafon-dsp/            # Pure DSP: biquads, dynamics, limiter, EQ, RNNoise/DFN wrappers.
│  │                        # No platform dependencies, no allocations in process(). criterion benchmarks.
│  ├─ macafon-engine/         # The chain graph, parameter state, lock-free exchange, A/B player, meters
│  ├─ macafon-backend/        # trait AudioBackend + shared device/stream types
│  ├─ macafon-backend-pipewire/   # The Linux implementation (pipewire-rs)
│  ├─ macafon-backend-windows/    # (M3) WASAPI, VB-CABLE, audio sessions, IPolicyConfig (the `windows` crate)
│  ├─ macafon-ipc/            # Protocol (serde types, protocol version), client/server
│  ├─ macafon-config/         # Configuration schema, migrations, presets
│  ├─ macafon-apps/           # Application classification, known-application database, icon lookup
│  └─ macafon-stt/            # Dictation: SpeechEngine trait, Whisper/Parakeet engines, VAD, model manifest
├─ apps/
│  ├─ macafond/               # Daemon: engine + backend + IPC server + tray + hotkeys
│  ├─ macafon-ui/             # Tauri v2 (src-tauri) + Svelte 5 + TypeScript (src)
│  ├─ macafon-stt/            # Dictation worker process, spawned by the daemon on demand (§8.4.3)
│  └─ macafon-cli/            # `macafon ctl …`
├─ packaging/               # deb, rpm, AUR PKGBUILD, AppImage, systemd unit, .desktop, icons, Windows NSIS hooks
├─ docs/                    # PRD, design, ADRs
└─ tools/latency-probe/     # Latency measurement utility (§14.3)
```

### 11.3 Key dependencies

| Purpose | Crate / technology |
|---|---|
| PipeWire | `pipewire` (pipewire-rs), `libspa` |
| AI NS Lite | `nnnoiseless` |
| AI NS Studio | `deep_filter` (DeepFilterNet, inference on `tract`) |
| Lock-free | `rtrb`, `atomic_float`, `arc-swap` (outside RT) |
| Async/control | `tokio` |
| IPC | `serde` + `serde_json` (MVP; a binary format if needed), `interprocess` |
| Config | `toml`, `serde`, `directories` |
| Windows (M3) | `windows` (WASAPI, Core Audio, COM), `rubato` (clock drift compensation) |
| Tray | Linux: `ksni`; Windows: `tray-icon` |
| Hotkeys | `ashpd` (xdg-portal GlobalShortcuts), `global-hotkey` (X11/Windows) |
| UI | Tauri v2, Svelte 5, TypeScript, Vite; Canvas 2D for EQ/meters |
| Logging | `tracing` (outside the RT thread) |
| RT checks | `assert_no_alloc` |
| FFT (spectrum, calibration) | `realfft` |
| Dictation (§8.4) | `whisper-rs` (whisper.cpp), sherpa-onnx through a Rust binding (chosen in D2), ONNX Runtime; `rubato` for 48 → 16 kHz; an HTTPS client with rustls and `sha2` for model downloads (prefer crates already in `Cargo.lock`) |

### 11.4 Audio engine

- Block processing: `process(&mut self, buf: &mut [f32])` on quantum-sized blocks; modules with a fixed
  frame (RNNoise, 480 samples) work through an internal FIFO and report their latency.
- Every module implements a trait:
  ```rust
  pub trait Processor: Send {
      fn prepare(&mut self, sample_rate: u32, max_block: usize, channels: usize); // outside RT, may allocate
      fn process(&mut self, io: &mut AudioBlock);  // RT: no allocations
      fn latency_samples(&self) -> u32;
      fn reset(&mut self);
      fn set_param(&mut self, id: ParamId, value: f32); // called in RT from the queue, applied with smoothing
      fn telemetry(&self) -> ModuleTelemetry;           // gain reduction, activity — read through atomics
  }
  ```
- Chain = `Vec<Box<dyn Processor>>`, built outside the RT thread; the swap is atomic, and the old chain is
  freed in a non-RT thread (through a "garbage" queue).
- Meters: the RT thread writes peak/RMS into atomics; the control loop reads them at 60 Hz and sends them
  to subscribed clients.
- A/B player: a separate microphone chain instance whose parameters are synchronized with the main one;
  it runs in the same data loop, and its output is a separate playback stream to the monitoring device.

### 11.5 IPC protocol (sketch)

```jsonc
// UI → daemon
{"id":1,"method":"mic.set_param","params":{"module":"compressor","param":"amount","value":0.55}}
{"id":2,"method":"apps.move","params":{"app_key":"bin:discord","channel":"voice"}}
{"id":3,"method":"meters.subscribe","params":{"targets":["mic.in","mic.out","ch.*"],"hz":60}}
// daemon → UI (events)
{"event":"apps.added","data":{"app_key":"bin:spotify","name":"Spotify","icon":"spotify","channel":"music","active":true}}
{"event":"meters","data":{"mic.in":[-21.3,-30.1],"mic.out":[-16.0,-22.4],"mic.gr.compressor":-4.1}}
```

The protocol version is part of the handshake; a UI of an incompatible version gets a clear error
(important during updates). The implemented protocol is described in
[daemon.md](development/daemon.md).

---

## 12. Platform implementation

### 12.1 Linux / PipeWire (M0–M2)

**Virtual microphone:**
- Preferred: one `pw_filter` node with input ports (linked to the physical microphone) and output ports,
  `media.class = "Audio/Source/Virtual"`, `node.name = "macafon_mic"`,
  `node.description = "Macafon Microphone"`. In-place processing in one graph cycle → no extra buffering.
- Fallback (if a filter node as a source misbehaves in some versions): a pair of streams, capture
  (`Stream/Input/Audio`, target = the physical microphone) + source (`Audio/Source`), in one process/graph
  driver, joined by a 1-quantum SPSC buffer.
- The choice follows the M0 spike with measurements.
- Linking to the physical microphone is explicit (`target.object` / manual links through the registry),
  with `node.dont-reconnect` logic under our control so that WirePlumber does not "relink" the input.

**Output channels:**
- A node per channel: `media.class = "Audio/Sink"`, `node.name = "macafon_ch_<id>"`,
  `node.description = "Macafon <Name>"`, stereo. Each channel's output → an internal mix → one playback
  stream to the real device (fewer nodes in the graph, a shared Master) — or a playback stream per channel
  with mixing in PipeWire. **Decision — [ADR-0003](adr/0003-linux-channels-pw-filter-sinks.md):** a
  `pw_filter` `Audio/Sink` node per channel, the outputs are linked to the device, PipeWire does the mixing,
  Master is a multiplier inside the channels.
- Moving application streams: setting `target.object` for the stream node through `metadata` (the
  `default` metadata object, as `pavucontrol`/`pw-metadata` do); WirePlumber relinks the stream without
  restarting it. The "application → channel" rule is applied when the stream appears in the registry
  (before the first buffer as far as possible; in a race — an instant move).
- Default sink: `default.configured.audio.sink` in the metadata → `macafon_ch_main` when the mixer is
  enabled; restored when it is disabled.
- Stream tracking: registry events for `media.class = "Stream/Output/Audio"`, the properties
  `application.name`, `application.process.binary`, `application.process.id`, `application.icon-name`,
  `pipewire.access.portal.app_id`, `media.role`.

**Why not PipeWire/LV2 modules (filter-chain):** configuration through files and restarts, no convenient
real-time exchange of meters and parameters, DeepFilterNet/A/B cannot be embedded. Our own process gives
full control and works on any distribution without installing system configs.

**M0 risk checks:** WirePlumber versions reacting differently to `target.object`; applications with
`node.dont-reconnect=true`; Flatpak applications; Chromium/Electron (Discord) with several streams;
behavior on `systemctl --user restart pipewire`.

### 12.2 Windows (M3)

> **Decision ([ADR-0004](adr/0004-windows-free-vb-cable.md), supersedes ADR-0001 and ADR-0002):** on Windows
> Macafon relies **only on the free [VB-CABLE](https://vb-audio.com/Cable/)** by VB-Audio — one virtual
> cable (`CABLE Input` → `CABLE Output`). There is no driver, APO plugin or helper service of our own. The
> paid/donation A+B and C+D cables are neither used nor advertised: everything must work on the one free
> cable. The user installs VB-CABLE from the official site; Macafon does not redistribute it, does not
> bundle it in the installer and does not download it.

One cable carries audio in one direction, so it is given entirely to the **virtual microphone** — the
product's main feature. The Windows mixer works without cables: channels are **application volume groups**
on top of Windows audio sessions (§12.2.2).

#### 12.2.1 Microphone — the virtual microphone on VB-CABLE

```
Physical mic ─► WASAPI capture ─► macafond: chain §6.3 ─► WASAPI render ─► CABLE Input
                                         ▲ meters / A/B / calibration               │ VB-CABLE
                                         │                                          ▼
                                   macafon (UI)              Discord / OBS / Zoom ◄─ CABLE Output = "Macafon Microphone"
```

| Task | Solution |
|---|---|
| Cable detection | By the VB-Audio hardware ID / driver, **not by the endpoint name** (names are localized and renamed). Exact IDs — in the M3.0 spike. Exactly one VB-CABLE cable is needed; if several VB-Audio devices are found (the user has A+B, etc.), Macafon uses only the base `VB-CABLE` |
| Capture | WASAPI shared, event-driven, from the physical microphone; the engine format (float32, 1–2 channels). For non-48 kHz — a resampler at the chain input (RNNoise/DFN run at 48 kHz) |
| Output | WASAPI shared, event-driven, into `CABLE Input`; mono is duplicated into L/R (the cable is stereo) |
| Clock drift | The microphone and the cable run on different clocks. Between capture and output — a ring buffer with adaptive resampling (drift compensation ±0.1%), without clicks and without accumulating latency |
| RT thread | Processing in the capture thread (MMCSS "Pro Audio"), without allocations or locks (AGENTS §4.1, NFR-S1…S4); the output thread only reads the ring buffer |
| Mute / Bypass | In the chain, as on Linux: mute — digital silence into the cable, bypass — Gain = 0 dB |
| A/B and calibration | The raw signal is available in the daemon (its own capture), so §6.4/§6.5 work as on Linux, without workarounds |
| NS Studio | DeepFilterNet in the daemon, as on Linux; on overload — fallback to Lite (NFR-S3) |
| Default microphone (MIC-012) | `CABLE Output` becomes the default microphone (the Console and Communications roles) through `IPolicyConfig`, as in EarTrumpet/SoundSwitch; the original defaults are remembered in a journal and restored on Quit / shutdown / uninstall. No administrator rights needed |
| Signing | Authenticode for the exe and installer — SignPath Foundation (free for OSS). No driver signing needed: VB-CABLE is signed by VB-Audio |

**Requirements:**

| ID | Requirement | Priority |
|---|---|---|
| WIN-MIC-001 | Without VB-CABLE installed, microphone processing is unavailable: the Microphone screen and onboarding show the installation step (WIN-MIC-002); meters, calibration and A/B still work (processing runs, but there is nowhere to output) | P0 |
| WIN-MIC-002 | An "Install VB-CABLE" onboarding step (after Welcome): the explanation "Macafon uses the free VB-CABLE driver as your processed microphone", an `Open download page` button (the official VB-Audio site), instructions (unpack → `VBCABLE_Setup_x64.exe` as administrator → reboot), a note that it is donationware and the author deserves support. After the reboot Macafon continues onboarding from this step | P0 |
| WIN-MIC-003 | In the UI the cable is called **Macafon Microphone**; the "Use it" step has the hint "In Discord, OBS, Zoom select **CABLE Output (VB-Audio Virtual Cable)**" with a picture, because the system device name stays foreign | P0 |
| WIN-MIC-004 | The cable belongs to Macafon: if another application (OBS, a game, VoiceMeeter) writes into `CABLE Input`, or the system default render device points at the cable, the audio mixes with the microphone. Doctor detects foreign sessions on `CABLE Input` and a default render on the cable and offers a one-click fix (move the default render back to a real device) | P0 |
| WIN-MIC-005 | Protection against a "missing microphone": if Macafon is not running, `CABLE Output` delivers silence. On Quit and shutdown the original default microphone is restored; the Quit dialog warns that applications that chose the cable explicitly will be left without audio. Autostart (AS-004) is on by default | P0 |
| WIN-MIC-006 | Hot-plug (MIC-005): the cable keeps existing; when the physical microphone disconnects, silence goes into it; on reconnect — an automatic return, as on Linux | P0 |
| WIN-MIC-007 | Cable format: 48 kHz. Doctor checks the format of `CABLE Input`/`CABLE Output` and the physical microphone, warns about resampling and shows where to change the format in the Windows sound settings | P0 |
| WIN-MIC-008 | Latency: Doctor shows the actual latency (capture + chain + cable buffer + output) and the recommended size of the VB-CABLE internal buffer (`VBCABLE_ControlPanel`). Target — NFR-L6 | P0 |
| WIN-MIC-009 | The cable is hidden in the Macafon UI: it is not shown in the physical microphone choice (MIC-003) or in the output device list (OUT-004) | P0 |
| WIN-MIC-010 | A "Rename to Macafon Microphone" option: a one-time action with a UAC prompt renames the `CABLE Output` endpoint to `Macafon Microphone`; the original name is saved and restored when Macafon is uninstalled | P2 |

**Differences from Linux:**

| Requirement | Linux | Windows |
|---|---|---|
| MIC-010 Virtual microphone | The daemon creates `Macafon Microphone` | `CABLE Output` from VB-CABLE; exists while the driver is installed, silent without Macafon |
| MIC-011 Format | Mono / a stereo option | Stereo (duplicated mono), the cable sets the format |
| Latency without NS | ≤ 3 ms (NFR-L1) | Higher: two WASAPI passes + the cable buffer (NFR-L6) |
| Several virtual microphones (M4) | Yes | No — the free VB-CABLE is a single cable |

#### 12.2.2 Windows mixer — volume groups without cables

Optional output-device controls (owner request, 2026-10-17) are separate from channel/Master FX:

| ID | Requirement | Priority |
|---|---|---|
| WIN-DAC-001 | Detect iBasso Jr. Macaron by USB VID/PID `262A:1A0B` and associate its HID interface with its render endpoint through the physical PnP ancestor. Controls appear only for connected supported devices: beside the current Mixer output and in Settings. No onboarding dependency, driver installation or administrator rights | P1 |
| WIN-DAC-002 | Match the official web console without EQ: hardware volume 0–100 in integer steps, balance L10–R10, gain Low/Mid/High, five digital filters (Fast Roll-off, LD Slow Roll-off, LD Fast Roll-off, Slow Roll-off, NOS), output Normal/Boost. Hardware volume remains separate from Master | P1 |
| WIN-DAC-003 | Read actual volume/balance on connection and refresh every 250 ms in the background controller using the Android UAC memory-read commands. A single-slider edit preserves the other channel setting read from hardware; verify volume writes by reading back. Physical-button changes update the UI without depending on short-lived notification reads. Filter/gain/output remain last-saved values and are never automatically applied at startup. The owner accepted no automatic restoration of unreadable original settings | P1 |
| WIN-DAC-004 | All USB operations run on a bounded background controller, outside audio and daemon control loops. Validate commands, wait for matching acknowledgements with bounded timeouts, invalidate cached values on partial write failure, retain last settings for display across disconnection, and report failures without interrupting normal audio | P1 |
| WIN-DAC-005 | The browser mock covers absent, unknown, acknowledged, disconnected and failed device states. Accessible controls use the existing design tokens and generated IPC types. Device settings show only the controls and actionable errors: no explanatory paragraphs, redundant status badges or separate Apply step (owner correction, 2026-10-17). No runtime website dependency, EQ, firmware flashing or permanent-profile writing | P1 |

There are no cables for channels, so application audio does not pass through Macafon. A channel on Windows
is a group of applications with a shared fader and mute: Macafon controls the volume of their Windows audio
sessions. The channel model, rules, auto-classification, D&D and hotkeys are the same as on Linux; there are
no effects on channels or Master.

```
Discord ─┐ session × (Voice 50% × app 100%)
Firefox ─┼─ session × (Main 30% × …)            ──► Windows mix -> system Master -> output device
Cider   ─┘ session × (Music 70% × …)
```

| ID | Requirement | Priority |
|---|---|---|
| WIN-CH-001 | Application auto-detection — `IAudioSessionManager2` + `IAudioSessionNotification` on all output devices; grouping by exe (APP-002), with Macafon's dedicated monitor session exposed separately as **SoundDeck** and its internal cable/preview sessions excluded (owner request, 2026-10-18); icons from the exe / AppX manifest (APP-003); system sounds — a pseudo-chip (APP-011) | P0 |
| WIN-CH-002 | Session volume = a cubic curve (`channel × application`) through `ISimpleAudioVolume`; channel mute affects sessions. Master mirrors the default Windows render endpoint through `IAudioEndpointVolume`, including external volume/mute changes, adopts Windows on startup/device changes, and never attenuates sessions a second time. Explicit Master edits persist as system volume on exit (owner decision, 2026-10-17). A change applies to all of the application's sessions, including new ones (APP-005) | P0 |
| WIN-CH-003 | A change of an application's volume in the Windows volume mixer (or by the application itself) is recognized by Macafon through `IAudioSessionEvents::OnSimpleVolumeChanged` (its own event-context GUID tells its own changes apart) and accepted as the application's new volume (CH-008), not overwritten | P0 |
| WIN-CH-004 | Meters: session `IAudioMeterInformation`; a channel meter is the maximum of its applications' peaks (CH-005), Master is the output device meter | P0 |
| WIN-CH-005 | Up to 8 channels, as on Linux (channels need no cables) | P0 |
| WIN-CH-006 | On Windows channels and Master have **no EQ/FX** (CFX-001…007): the EQ/FX buttons are hidden, the channel panel has a Channel tab; the UI explains once "Channel effects are available on Linux" | P0 |
| WIN-CH-007 | Output device (OUT-001…003): changing the system default render device through `IPolicyConfig`; the priority list (OUT-002) — Macafon switches the default on connect/disconnect. Applications that chose a device themselves are left alone | P0 |
| WIN-CH-008 | Recovery: the original session volumes and mutes changed by Macafon are saved in a journal and given back when the mixer is disabled, on Quit, after a daemon crash (on the next start) and on uninstall | P0 |
| WIN-CH-009 | An application cannot be redirected "into a channel" at the audio level — a channel does not change the application's device. Moving a chip only changes which fader controls the application | P0 |
| WIN-CH-010 | Per-channel effects without extra cables (for example, WASAPI process loopback) — outside 1.x; revisited in ADR-0004 | P2 |
| WIN-CH-011 | Settings shows Windows sessions as the active mixer mode and a disabled VB-CABLE (Soon) option. A future cable mixer requires a separate user-installed cable; no cable mixer or effects are enabled yet (owner decision, 2026-10-17) | P0 |
| WIN-CH-012 | A session of an app Macafon has no settings for yet gets the default channel's volume on the mixer worker the moment the session appears, before the app is classified; it is then corrected to its channel or restored if unmanaged. A new app's first sound (a notification) must not play at full volume past its channel (owner report, 2026-10-19) | P0 |

**Accepted limitations of this approach:** one virtual microphone; higher microphone latency than on Linux;
a foreign device name in applications (until WIN-MIC-010); no channel or Master effects on Windows;
`CABLE Output` is silent without Macafon running; dependency on a third-party driver.

#### 12.2.3 Other

| Task | Solution |
|---|---|
| Tray / autostart | `tray-icon`; daemon autostart at logon through `HKCU\…\Run` (no UAC) (AS-004) |
| IPC | The named pipe `\\.\pipe\macafon-<user SID>` with a current-user ACL instead of a Unix socket |
| Hotkeys | `global-hotkey` (RegisterHotKey), plus `WH_KEYBOARD_LL` for side-specific Ctrl/Alt, including release events for push-to-talk |
| Installer | MSI (WiX) through the Tauri bundler, **a per-user install without administrator rights** (no services or system components). VB-CABLE is **not** part of the installer (WIN-MIC-002). Implemented as a per-user NSIS installer, because the Tauri MSI installs per machine ([windows-m3.md](development/windows-m3.md)) |
| Uninstall | Restores default devices and session volumes from the journals (WIN-MIC-005, WIN-CH-008); does not remove VB-CABLE — explains how to remove it yourself |
| Our own virtual device driver | Postponed; revisited if there is a budget for signing (then several virtual mics and channels with effects, as on Linux) |


---

## 13. Configuration storage and presets

| What | Where (Linux) | Format |
|---|---|---|
| Main config | `$XDG_CONFIG_HOME/macafon/config.toml` | TOML, a `schema_version` field, automatic migrations + a `config.toml.bak` backup before migrating |
| User microphone presets | `$XDG_CONFIG_HOME/macafon/presets/mic/*.json` | JSON (`.macafon-preset.json` on export) |
| Channel EQ presets | `$XDG_CONFIG_HOME/macafon/presets/eq/*.json` | JSON |
| Application rules | In `config.toml`, the `[[apps.rules]]` section (`key`, `channel`, `managed`) | TOML |
| Logs | `$XDG_STATE_HOME/macafon/macafond.log` (rotation 5 × 2 MB) | text |
| Socket | `$XDG_RUNTIME_DIR/macafon/macafond.sock` | — |

Windows — `%APPDATA%\Macafon\…`, `%LOCALAPPDATA%\Macafon\logs`.

Requirements:
- CFG-001 (P0) Config writes are atomic (temp + rename), with a 500 ms debounce after the last change.
- CFG-002 (P0) A corrupted config → load the defaults + keep the broken file as `config.toml.broken-<ts>` + a toast.
- CFG-003 (P1) The config is human-readable and documented (for enthusiasts, dotfiles).

Example:
```toml
schema_version = 1

[mic]
device = "alsa_input.usb-Blue_Microphones_Yeti-00.analog-stereo"
input_channel = "left"
preset = "Discord Clear"
muted = false
bypass = false

[mic.modules.noise_suppression]
enabled = true
engine = "lite"
intensity = 0.8

[mixer]
enabled = true
output_priority = ["alsa_output.usb-SteelSeries_Arctis_Nova", "alsa_output.pci-0000_00_1f.3.analog-stereo"]
default_channel = "main"

[[mixer.channels]]
id = "voice"
name = "Voice"
color = "violet"
volume = 0.5
[mixer.channels.fx.compressor]
enabled = true
amount = 0.4

[[apps.rules]]
key = "bin:discord"
channel = "voice"

[hotkeys]
mic_mute_toggle = "Ctrl+Alt+M"
dictation_hold = "Ctrl+Win"
dictation_toggle = "Ctrl+Win+Space"
dictation_paste_last = "Shift+Alt+Z"
dictation_copy_last = "Shift+Alt+X"
dictation_hold_copy = "Ctrl+Win+Shift"

[dictation]
enabled = false
engine = "whisper"            # whisper | parakeet
model = "whisper-large-v3-turbo-q5_0"
language = "layout"           # layout | fixed | auto
fixed_language = "uk"         # en | uk | ru
tap = "clean"                 # raw | clean | processed
mute_mic_while_dictating = true
insert = "paste"              # paste | type
trailing_space = true
fullscreen_games = "copy"     # insert | copy | ignore
unload_after_s = 180          # 0 = immediately, -1 = never
threads = 0                   # 0 = max(2, physical cores / 2)
game_mode = true
pre_roll = true
keep_history = true
vocabulary = ["Macafon", "PipeWire"]
models_dir = ""               # empty = %LOCALAPPDATA%\Macafon\models\stt

[dictation.popup]
look = "flow"                 # flow | analog
position = "bottom"           # bottom | top
offset_pct = 0
```

---

## 14. Testing and quality

### 14.1 Automated tests

| Level | What | Tools |
|---|---|---|
| DSP unit | Filter responses against analytic formulas (±0.1 dB), static compressor/expander curves, no NaN/denormals, a null test "everything off = bit-exact", determinism | `cargo test`, `proptest` |
| DSP golden | Reference WAVs (voice + noise: fan, keyboard, street) → processing → comparison with a golden output (with tolerance) | our own harness |
| RT safety | Running all modules and preset changes under `assert_no_alloc` | CI |
| Performance | `criterion` benchmarks per module and chain; a regression > 10% breaks CI | CI |
| Engine/IPC | Integration tests of the protocol and config migrations | `cargo test` |
| PipeWire integration | Headless PipeWire + WirePlumber in a CI container: node creation, moving streams (`pw-play` as an "application"), nodes disappearing when the daemon stops | GitHub Actions |
| UI | Component tests (Vitest), e2e (WebDriver through `tauri-driver`) for onboarding and D&D | CI |

These checks run locally (the headless PipeWire tests through `tools/tests/*.py`). The owner authorized
a manual GitHub Actions Windows installer build on 2026-10-17; other cloud CI remains out of scope per
[AGENTS.md](../AGENTS.md).

### 14.2 Manual testing

The §10.5 matrix × scenarios US-1…US-10 before every release; the checklist is in
[`docs/qa/release-checklist.md`](qa/release-checklist.md).

### 14.3 Latency measurement

`tools/latency-probe`: generates an impulse/MLS into a "fake microphone" node, catches it at the output of
`Macafon Microphone` and measures the difference from the direct path; the same for output channels. The
results are published in the README/release notes. Additionally — a physical loopback test (a cable from
the sound card's output to its input).

### 14.4 Application set for routing checks

Firefox, Chromium, Discord (native + Flatpak), Vesktop, Telegram Desktop, Spotify (Flatpak/deb), Cider,
Steam + a Proton game, a native game (SDL), mpv, OBS (as a consumer of the virtual microphone), Zoom.

## 15. Distribution

| Channel | Priority | Note |
|---|---|---|
| GitHub Releases: `.deb`, `.rpm`, AppImage | P0 | Tauri bundler + systemd user unit |
| AUR (`macafon`, `macafon-bin`) | P0 | |
| Flatpak (Flathub) | P2 | The daemon needs access to PipeWire and the registry of all nodes — the sandbox needs research |
| Windows: a per-user installer (no administrator rights) | P0 (M3) | Authenticode signing through SignPath Foundation (free for OSS); the user installs VB-CABLE themselves |
| Auto-update | P1 | Only a new-version notification (opt-in); updating through the package manager / installer |

Windows 11 is the working platform; Linux remains experimental (owner decision, 2026-10-17).
The manual `Windows 11 build` GitHub Actions workflow builds the x64 per-user NSIS installer and uploads
it with SHA-256 checksums as a 30-day artifact. It uses the existing Windows packaging script; no driver,
installation on the runner, signing or automatic release publishing is added. Other checks and Linux
packaging remain local per [AGENTS.md](../AGENTS.md).

## 16. Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| WirePlumber overrides `target.object` / restores old stream routes | Medium | High | An M0 spike on several versions; if needed — our own links + a WirePlumber script/policy shipped with the package |
| `pw_filter` source latency higher than expected | Low | High | Measurement in M0, a fallback design, honest latency display |
| DeepFilterNet too heavy on weak CPUs | Medium | Medium | Lite by default, an overload detector with automatic fallback |
| Global hotkeys on Wayland unavailable in some DEs | High | Medium | Portal + X11 + a CLI fallback with ready commands |
| GNOME without AppIndicator — no tray | High | Medium | Instructions, control by relaunching, hotkeys |
| Double processing (Discord NS/AGC + Macafon) ruins the sound | High | Medium | Hints in onboarding, Doctor |
| Applications that grab the device directly through ALSA (without PipeWire) | Low | Low | Document as a limitation |
| Windows: users do not install VB-CABLE (an extra step + a reboot) — the microphone does not work without it | Medium | High | A clear onboarding step (WIN-MIC-002), continuing after the reboot; the mixer and the sound test work without the cable |
| Windows: `CABLE Output` is silent when Macafon is not running | Medium | High | Autostart by default, restoring the default microphone on Quit, a warning in the Quit dialog (WIN-MIC-005) |
| Windows: the cable is already taken by OBS/VoiceMeeter/a game — foreign audio in the microphone | Medium | High | Doctor detects foreign sessions and a default render on the cable, a one-click fix (WIN-MIC-004) |
| Windows: VB-CABLE latency higher than the target | Medium | Medium | Measurement in M3.0, the recommended cable buffer in Doctor (WIN-MIC-008) |
| Windows: clock drift between the microphone and the cable — clicks or growing latency | Medium | High | Adaptive resampling in the ring buffer, a 1 h soak test in M3 |
| Windows: channels without effects disappoint Sonar users | High | Medium | Honest about it in onboarding and the UI (WIN-CH-006); per-channel effects — revisited in ADR-0004 |
| Windows: session volume is changed by the application or the Windows mixer | High | Low | Accept it as the application's new volume (WIN-CH-003) |
| Windows: VB-CABLE changes or is discontinued | Low | High | Detection by hardware ID, a "virtual microphone" abstraction in the backend; our own driver as a long-term alternative |
| Windows: the undocumented `IPolicyConfig` changes between builds | Medium | Medium | GUID versions per build (as in EarTrumpet/SoundSwitch); without it — instructions to choose the device manually |
| WebKitGTK is slow on animations / Canvas | Medium | Medium | Canvas 2D with an FPS limit, disabling animations, profiling in M1 |
| Dictation hurts game performance | Medium | High | Off by default, worker only on demand, BELOW_NORMAL + EcoQoS, thread limit, game mode forces CPU and immediate unload (STT-008); measured in D8 |
| Whisper mixes up Ukrainian and Russian | High | Medium | The layout-based policy (§8.4.4), Fixed mode and the language hotkey |
| Parakeet's automatic language choice is wrong for UK/RU | Medium | Medium | Documented as Auto only; Whisper stays the default engine for UK/RU |
| Injected input (`SendInput`) flagged by anti-cheats | Low | High | Full-screen games default to Copy only (STT-012); no input is injected when the user does not dictate |
| Hugging Face files move or change | Low | Medium | URLs pinned to a commit and SHA-256; import of a local file with the same hash |
| Installer growth from the STT runtimes | High | Low | Accepted deviation from NFR-R6 (§17 Q9); measured in D1/D2; GPU runtimes stay out until approved |
| `Ctrl+Win` chords conflict with other tools or open Start | Medium | Low | Win release swallowed by the LL hook; conflict detection; easy rebinding (§17 Q8) |

## 17. Open questions

| # | Question | Recommendation | Who decides |
|---|---|---|---|
| Q1 | **Decided by the owner:** GPL-3.0-only for the application, MIT/Apache-2.0 for `macafon-dsp` | License texts in the repository | Product owner |
| Q2 | ~~Final name~~ — **decided:** Macafon (the working name was micEQ). Logo — decided (Retro, see the design system) | — | Product owner |
| Q3 | ~~Budget for signing a Windows driver~~ — **decided:** no driver of our own, only the free VB-CABLE ([ADR-0004](adr/0004-windows-free-vb-cable.md)). Our own driver — revisit when there is a budget | — | — |
| Q7 | ~~Output channels on Windows~~ — **decided:** the only free cable goes to the microphone, channels are volume groups without effects ([ADR-0004](adr/0004-windows-free-vb-cable.md)) | — | — |
| Q4 | Is a separate channel/fader for the microphone needed in the mixer (sidetone — hearing yourself) | Postpone together with live monitoring | Product |
| Q5 | Should community presets be stored (a catalog in the repository) | P2: a `community-presets/` folder + import by link | Product |
| Q6 | aarch64 Linux support in 1.0 | P1, if the build setup allows | Development |
| Q8 | ~~Default dictation shortcuts~~ — **decided:** Wispr Flow's Windows defaults plus copy/paste actions (STT-003) | — | — |
| Q9 | ~~Installer growth from the STT runtimes~~ — **decided:** accepted, recorded in NFR-R6 | — | — |
| Q10 | ~~Delete dictation models on uninstall~~ — **decided:** yes by default, with a checkbox to keep them (§8.4.6) | — | — |

## 18. Success metrics

There is no telemetry, so the metrics come from tests and the community:

| Metric | Target for 1.0 |
|---|---|
| Added latency (latency-probe) | NFR-L1…L3 met and published |
| CPU/RAM (benchmark script) | NFR-R1…R5 met |
| Onboarding time (usability, n ≥ 5) | Median ≤ 2 min |
| A/B blind test (n ≥ 10 listeners): "processed is better" on the Natural preset with a noisy recording | ≥ 80% |
| Crash / xrun bugs in GitHub Issues in the first month after release | 0 critical ones open > 7 days |
| Routing: applications from §14.4 | 100% move correctly and are remembered |

## 19. Roadmap

| Stage | Content |
|---|---|
| **M0 · Spike** (1–2 weeks) | A PoC of the virtual microphone on `pw_filter` + RNNoise, latency measurement, a PoC of moving an application stream into a virtual sink through metadata, a DeepFilterNet check in RT. The outcome — an ADR with the chosen design |
| **M1 · MVP Microphone** | Daemon, the 8-module chain, presets, calibration, recording + A/B, the Microphone screen, onboarding, tray, autostart, deb/rpm/AppImage/AUR |
| **M2 · 1.0 Mixer** | Channels, auto-detection + D&D + rules, channel EQ/FX, output device selection and priority, hotkeys (portal/X11/CLI), Doctor |
| **M3 · Windows** | See the plan in §19.1 |
| **M4 · Streamer** | Several virtual microphones (different chains from one input), Stream mix (a separate channel mix for OBS), ducking (lowering Main/Music when someone speaks in Voice), live self-monitoring |
| **M5 · Pro** | A spectrum on the EQ, module reordering, CLAP/LV2/VST3 hosting, splitting the browser by tabs, Spatial audio (HRTF), channel loudness normalization, AutoEQ headphone profiles, community presets |
| **M6** | UI localization (RU and others), aarch64, Flatpak, remote control (Stream Deck / phone) |

M4 on Windows is limited by the one free cable: several virtual microphones and Stream mix are not
available there (ADR-0004).

Dictation (§8.4) is an owner-requested Windows feature outside the milestone order, planned in §19.2.

### 19.1 Windows port plan (M3)

The basis is §12.2 and [ADR-0004](adr/0004-windows-free-vb-cable.md): only the free VB-CABLE (one cable) for
the virtual microphone, the mixer as volume groups without effects. No driver, APO or services of our own.
Checks run locally on a Windows test machine (Windows 10 22H2 and Windows 11 24H2/25H2 with VB-CABLE
installed). A manual GitHub Actions workflow also builds the Windows x64 installer (owner authorization,
2026-10-17); live audio checks remain local. Progress — [windows-m3.md](development/windows-m3.md).

| Step | Content | Done when |
|---|---|---|
| **M3.0 · Spike** (1–2 weeks) | The VB-CABLE driver hardware ID and reliable cable detection; a WASAPI capture → `macafon-dsp` → `CABLE Input` prototype; latency measurement with different `VBCABLE_ControlPanel` buffers (`tools/latency-probe` for Windows); measuring the clock drift between the microphone and the cable; `IPolicyConfig` (default capture/render) on the target builds; `ISimpleAudioVolume` + session events; named pipe IPC | Latency and drift numbers recorded in ADR-0004, NFR-L6 refined; `IPolicyConfig` GUIDs per build known |
| **M3.1 · Platform layer** | A shared `trait AudioBackend` in `macafon-backend` instead of Linux-specific host traits; `macafon-backend-windows` (the `windows` crate): devices and hot-plug (`IMMNotificationClient`), the microphone engine (an MMCSS capture thread + a ring buffer with drift compensation + an output thread), mute/bypass, A/B, calibration, NS Studio; IPC over a named pipe; `%APPDATA%`/`%LOCALAPPDATA%` paths; the default device journal | The daemon on Windows delivers processed audio into `CABLE Output` and survives a microphone disconnect and a format change; ring buffer/drift tests under `assert_no_alloc`; a one-hour soak without xruns |
| **M3.2 · Platform capabilities in the protocol** | `hello` returns `platform` and capability flags (`virtual_mic_device`, `channel_fx`, `multiple_virtual_mics`, `hotkey_backend`…); the UI hides what is unavailable (channel EQ/FX, the stereo option, renaming) by flags, not by an OS check in the UI; the mock daemon gets a `?platform=windows` mode | The UI in Windows mock mode shows the right screens; TS types generated |
| **M3.3 · Microphone in the UI and onboarding** | The "Install VB-CABLE" step continuing after a reboot (WIN-MIC-002), "Use it" with `CABLE Output` (WIN-MIC-003), the default microphone toggle, a warning in Quit (WIN-MIC-005), Doctor: cable found, 48 kHz format, buffer and latency, foreign sessions on `CABLE Input`, default render on the cable (WIN-MIC-004/007/008) | Onboarding goes through on a clean system from installing VB-CABLE to audio in Discord |
| **M3.4 · Mixer** | Sessions and grouping by exe, icons from exe/AppX, volume/mute through `ISimpleAudioVolume` (`channel × application`), Master through `IAudioEndpointVolume`, accepting external volume changes, session meters, output device selection and priority, a journal and volume restore (WIN-CH-001…009) | Applications from §14.4 (Windows versions) are grouped, faders and mute work, volumes are restored after Quit/a crash |
| **M3.5 · Background and control** | Tray (`tray-icon`), autostart through `HKCU\…\Run`, single instance, `global-hotkey` hotkeys with push-to-talk, `macafon ctl` over the named pipe, Windows notifications | Hotkeys work with the window closed; HK-004 CLI commands work |
| **M3.6 · Package and release** | A per-user installer (implemented with NSIS instead of the Tauri bundler + WiX MSI) without VB-CABLE, SignPath signing, uninstall restoring default devices and volumes; a Windows release checklist; a test matrix: USB microphones, Realtek, USB/Bluetooth headsets; Discord, OBS, Zoom, Teams, Telegram, games with anti-cheats (Vanguard, EAC, BattlEye, FACEIT) | The checklist passes on Windows 10 22H2 and Windows 11 24H2/25H2 |

### 19.2 Dictation plan (STT)

Requirements: §8.4 and §9.9; decision: [ADR-0005](adr/0005-local-dictation-worker.md). Both engines are
built; there is no engine bake-off step. Checks follow AGENTS §3: targeted tests per step, `cargo test
--workspace` only for the steps touching shared crates (`macafon-ipc`, `macafon-config`,
`macafon-engine`). Every step ends with a working, committable state.

| Step | Content | Done when |
|---|---|---|
| **D0 · Contracts and mock** | `[dictation]` config schema with defaults; IPC types (state, models, progress, results, config) and TS generation; `Capabilities::dictation`; mock daemon: fake models, downloads, worker states, levels, transcripts; the sidebar item with an empty Dictation screen | The UI in mock mode shows the screen and every `dictation.state`; `pnpm check` passes; config round-trip test |
| **D1 · Worker + Whisper** | `crates/macafon-stt` (`SpeechEngine`, `TranscribeOptions`, `Transcript`, the model manifest), `WhisperEngine` (whisper.cpp, Silero VAD, `no_speech`, hallucination filter, `initial_prompt`, restricted language probabilities); `apps/macafon-stt` with the pipe protocol, Job object, priority, EcoQoS, thread limit; `macafon stt transcribe` for local WAVs (STT-021) | EN/UK/RU WAVs transcribe through the CLI; RAM and time recorded for NFR-R10/NFR-L7; protocol round-trip and hallucination-filter tests |
| **D2 · Parakeet** | `ParakeetEngine` on sherpa-onnx with its Silero VAD; the binding is chosen here (official crate or `sherpa-rs`), ONNX Runtime packaged next to the worker | The same WAVs transcribe with `--engine parakeet`; measurements recorded; installer size delta measured (Q9) |
| **D3 · Model manager** | Pinned Hugging Face manifest, background download with `Range` resume, SHA-256, free-space check, cancel, delete, import, progress events; the models folder with move (STT-022); uninstall prompt (Q10) | Download → verify → ready works on a clean profile; a corrupted `.part` is rejected; hash verification test with a small fixture |
| **D4 · Daemon pipeline** | RT tap with the atomic gate and 2 s `rtrb`, pre-roll ring, the STT thread with the resampler, streaming to the worker during recording, worker lifecycle (spawn on press, load in parallel, unload timer, crash handling), game mode, mute while dictating | Tap and resampler tests under `assert_no_alloc`; resampler response test; feature off leaves no tap/worker (NFR-R9); a crash of the worker leaves audio untouched |
| **D5 · Hotkeys, language, insertion** | `dictation_*` actions with HK-009 rules and the STT-003 defaults (Start suppression, double tap → hands-free), paste/copy last and push-to-talk to clipboard, Esc, the layout reader (UWP/console cases), the §8.4.4 policy, Paste/Type insertion with clipboard restore and history exclusion, focus-change and UIPI fallbacks, full-screen games policy, history in memory | Table tests for the language policy (including "en layout + Russian speech"); manual run in Notepad, Chrome, Discord, Telegram, Word, an elevated window and a full-screen game |
| **D6 · Popups and quick access** | Flow and Analog looks in the HK-006 Direct2D popup thread with all §8.4.7 states, level bands and VU ballistics; the Dictation tile in the tray panel and overlay; the tray menu item | Both looks go through every state in a manual run; the VU ballistics test (300 ms to 99 %, overshoot ≤ 2 %); screenshots |
| **D7 · Dictation screen** | §9.9.1 in full: header toggle and status, shortcut block, cards 01–07, picture buttons with Preview, TRY IT and history, i18n strings, keyboard and ARIA | Mock-mode screenshots at 1180×760 of the empty, downloading, ready, listening and error states; relevant Vitest files |
| **D8 · Hardening and package** | Installer ships the worker and runtime DLLs; About licenses; 1 h soak with dictation every minute alongside the microphone chain; a full-screen game FPS comparison: feature off / on idle / dictating; release checklist items | No xruns attributable to dictation; no FPS change with the feature on and idle; checklist passes on Windows 11 |

## 20. Glossary

| Term | Meaning |
|---|---|
| **Virtual cable / device** | A software audio device that applications see as an ordinary microphone/speaker |
| **VB-CABLE** | VB-Audio's free (donationware) driver with one virtual cable: audio played into `CABLE Input` reaches applications as the `CABLE Output` microphone |
| **WASAPI** | Windows Audio Session API — the low-level Windows audio API |
| **Quantum** | The PipeWire processing block size in samples (128 @ 48 kHz = 2.67 ms) |
| **Xrun** | A skipped audio cycle (a click/crackle) because processing did not finish in time |
| **HPF** | High-pass filter — cuts low frequencies |
| **Downward expander** | Attenuates the signal below a threshold (a soft gate) |
| **De-esser** | Suppresses harsh hissing "s/sh" sounds |
| **Look-ahead** | "Looking ahead" in dynamics processing at the cost of latency |
| **SNR** | Signal-to-noise ratio |
| **LUFS** | A unit of perceived loudness |
| **SNI** | StatusNotifierItem — the Linux tray icon standard |
| **RT thread** | A real-time thread that processes audio; it must never block |
| **STT** | Speech to text — the dictation feature (§8.4) |
| **VAD** | Voice activity detection — finds the speech parts of a recording |
| **VU meter** | A volume-unit meter with slow, averaging needle ballistics (300 ms), used by the Analog popup |
