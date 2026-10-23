<p align="center">
  <img alt="Macafon logo: a sound wave inside a ribbon" src="packaging/icons/hicolor/256x256/apps/macafon.png" width="128">
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/branding/macafon-wordmark-white.svg">
    <img alt="Macafon" src="assets/branding/macafon-wordmark-black.svg" width="280">
  </picture>
</p>

<p align="center">
  <b>Microphone processing, per-app mixer, SoundDeck, local dictation and display profiles for Windows 11.</b>
</p>

<p align="center">
  <img alt="License: GPL-3.0-only" src="https://img.shields.io/badge/license-GPL--3.0--only-blue">
  <img alt="Windows 11: working" src="https://img.shields.io/badge/Windows_11-working-success">
  <img alt="Linux: experimental" src="https://img.shields.io/badge/Linux-experimental-orange">
</p>

## Features

**Microphone.** Your microphone goes through Gain → High-pass → noise suppression → Expander → EQ →
Compressor → De-esser → Limiter and comes out as a virtual microphone for Discord, OBS, Zoom or games.

- One-click calibration tunes the chain to your room and voice.
- Record a clip and switch **Before ⇄ After** to hear every change.
- AI noise suppression runs locally: *Lite* (RNNoise) or *Studio* (DeepFilterNet 3).
- Built-in and custom presets, a fallback microphone, per-microphone settings.

![Microphone screen](docs/images/microphone.png)

**Mixer.** Apps appear automatically; drag them between channels (Main, Voice, Music, up to eight) to
control their volume with one fader. Pick the output device or a priority list for automatic switching.
SoundDeck's headphone playback is an app of its own, so it goes into any channel too.

![Mixer screen](docs/images/mixer.png)

**SoundDeck.** Play sounds into your voice chat — you hear them in your headphones too.

- Drop audio files on the window or into the library folder — `Music\Macafon Sounds` or any folder on
  any drive. Each subfolder is a category: new folders show up in the app, and moving a sound to
  another category moves its file.
- Sounds are loudness-normalized: at 100 % they are about as loud as your voice, and a limiter keeps
  the voice chat from clipping.
- MP3, WAV, OGG, FLAC, M4A, AAC. Search, sort, favorites, per-sound volume and hotkeys; large libraries
  load in the background.
- Damaged or unsupported files are found automatically and moved to an **Unplayable** folder: they
  stay out of All sounds, and you see them in one list to delete.
- Your voice processing is not applied to sounds. Optionally mute your voice while a sound plays.

![SoundDeck screen](docs/images/sounddeck.png)

**Dictation.** Hold `Ctrl+Win`, speak, release — the text appears in the window you were typing in.
Free and fully local: speech is recognized on your own computer, works offline once the model is
downloaded, and your voice and text never leave it. A short guide walks you through it the first time.

- Two local engines: **Whisper** (whisper.cpp, about a hundred languages) and **Parakeet** (NVIDIA
  Parakeet TDT v3, fast on the CPU for European languages). Whisper runs on any GPU through Vulkan —
  ten seconds of speech in about a third of a second — and falls back to the CPU. Models are
  downloaded from Hugging Face only when you click *Get*, checked by SHA-256 and kept in any folder
  you choose.
- Your languages: Macafon finds the keyboard layouts installed on your computer and follows the
  layout of the window you dictate into. Or fix any language Whisper knows, or let it detect one.
- A vocabulary of names and terms recognizers often get wrong (GitHub, PostgreSQL, Wi-Fi…) is
  included; add your own.
- `Ctrl+Win+Space` dictates hands-free, `Shift+Alt+Z` pastes the last result again, `Shift+Alt+X`
  copies it, `Ctrl+Win+Shift` dictates to the clipboard only.
- Costs nothing while unused: the speech worker starts on the key press, runs at low priority and
  unloads after a few minutes. Your voice chat does not hear you dictate.
- Two small popups that show only your voice: a pill with level bars, or a vintage VU meter.

![Dictation screen](docs/images/dictation.png)

**Displays.** Save your monitor setups as profiles — "Work" with every screen, "Game" with only the
main monitor at 165 Hz with HDR, "Movie" with only the TV — and switch between them with one click,
a hotkey, the tray, the overlay or `macafon ctl display`.

- A profile keeps which monitors are on, the main one, position, resolution, refresh rate, rotation,
  duplicate or extend, and HDR. Save the current layout in one click, then fine-tune any monitor
  without recapturing.
- Monitors are recognized by their EDID, not by port or driver ids, so profiles survive reboots,
  driver updates and plugging a cable into another port. Missing monitors are skipped and named.
- Never stuck with a black screen: every layout is validated first, and on a new set of monitors
  **Keep these display settings?** brings the old layout back after 15 s unless you keep it.
  `Ctrl+Alt+Shift+F12` turns every monitor on, `Ctrl+Alt+Shift+F11` returns the previous layout.
- Optionally a profile moves Macafon's sound too — "Movie" plays through the TV.
- No administrator rights. Profiles change Windows display settings on purpose and stay after
  Macafon quits.

![Displays screen](docs/images/displays.png)

**Always at hand.** Global hotkeys (push-to-talk, mute, volume, sounds; left and right Ctrl/Alt are
told apart) work with the window closed and show a small on-screen popup. The tray icon opens a quick
panel; `LCtrl+LAlt+Z` opens the **quick controls overlay** with volumes, switches, SoundDeck and
dictation and display profiles. Choose in Settings what it shows — including an output device
switcher and a live microphone meter — whether side panels open to the right, left or below, and whether the overlay
stays in place or keeps itself centered. **Doctor** checks your audio setup.

<details>
<summary>More screenshots</summary>
<br>

<p align="center">
  <img alt="Quick controls overlay" src="docs/images/hotkey-overlay.png" width="720"><br>
  <sub><b>Quick controls overlay</b> (<code>LCtrl+LAlt+Z</code>): channel volumes, switches, display profile and preset over any app. The arrows on Dictation, History and Sounds open their panels.</sub>
</p>

<p align="center">
  <img alt="Overlay with SoundDeck" src="docs/images/hotkey-overlay-sounds.png" width="860"><br>
  <sub><b>Overlay with SoundDeck</b>: search, favorites, categories, a star on every pad and one click to play.</sub>
</p>

<p align="center">
  <img alt="Displays: keep these display settings?" src="docs/images/displays-pending.png" width="720"><br>
  <sub><b>Displays → Keep these display settings?</b>: a new layout reverts by itself unless you keep it.</sub>
</p>

<p align="center">
  <img alt="Tray quick panel" src="docs/images/tray-panel.png" width="360"><br>
  <sub><b>Tray panel</b> (right-click the tray icon): quick toggles, preset and volumes.</sub>
</p>

<p align="center">
  <img alt="Settings: quick controls overlay" src="docs/images/settings.png" width="720"><br>
  <sub><b>Settings → Quick controls overlay</b>: drag parts to arrange the overlay, or into Hidden to remove them.</sub>
</p>

<p align="center">
  <img alt="Settings: SoundDeck folder" src="docs/images/settings-sounddeck.png" width="720"><br>
  <sub><b>Settings → SoundDeck</b>: use any folder on any drive as the sound library.</sub>
</p>

</details>

## Install on Windows 11

1. Download `Macafon_*_x64-setup.exe` from the [latest release](https://github.com/Mar1kLab/Macafon/releases/latest).
   No administrator rights needed.
2. Install the free [VB-CABLE](https://vb-audio.com/Cable/) driver (run `VBCABLE_Setup_x64.exe` as
   administrator, then reboot). Macafon uses it as your virtual microphone.
3. Open Macafon and follow the setup: microphone, calibration, mixer.
4. In Discord, OBS or Zoom select **CABLE Output (VB-Audio Virtual Cable)** as the microphone.

Macafon must keep running in the tray — without it the cable is silent. Uninstalling restores your
original audio devices and app volumes.

## Linux (experimental)

The PipeWire backend creates its own virtual microphone and supports channel EQ and effects, but is
less tested. SoundDeck is Windows-only for now. Requires PipeWire ≥ 1.0 and WirePlumber ≥ 0.5.

```sh
sudo apt install build-essential pkg-config libclang-dev libpipewire-0.3-dev pipewire-bin \
                 libwebkit2gtk-4.1-dev libgtk-3-dev libdbus-1-dev
corepack pnpm -C apps/macafon-ui install --frozen-lockfile
python3 tools/package-linux.py     # deb, rpm and AppImage in target/release/bundle/
```

## Command line

```sh
macafon ctl status
macafon ctl preset broadcast            # natural, discord-clear, noisy-room, broadcast, warm-podcast, raw
macafon ctl mic mute toggle
macafon ctl channel voice volume +5
macafon ctl sounddeck list               # SoundDeck
macafon ctl sounddeck play "Memes/airhorn.mp3"
macafon ctl sounddeck stop
macafon ctl dictation toggle            # on, off, toggle
macafon ctl display                     # display profiles
macafon ctl display apply <id>          # also next, revert, all-on, save "Work"
macafon ctl doctor
```

## Build from source (Windows)

Needs stable Rust, Node.js 24, Python 3.12+, CMake, LLVM/libclang (set `LIBCLANG_PATH` if it is not
found) and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/#windows). CMake and
libclang build the dictation engines (whisper.cpp, sherpa-onnx); the GPU build of Whisper also needs
the [Vulkan SDK](https://vulkan.lunarg.com/) (`VULKAN_SDK`, for `glslc`).

```powershell
corepack enable
corepack pnpm -C apps/macafon-ui install --frozen-lockfile
python tools/package-windows.py            # installer in target/release/bundle/nsis/
python tools/package-windows.py --install  # build and install
```

The UI also runs in a browser with a mock daemon, no audio stack needed:

```sh
corepack pnpm -C apps/macafon-ui dev       # http://127.0.0.1:1420/?platform=windows
```

Contributors: start with [AGENTS.md](AGENTS.md). Requirements are in the [PRD](docs/PRD.md), the
visual language in the [design system](docs/design/design-system.md), architecture decisions in
[docs/adr](docs/adr/).

| Path | Purpose |
|---|---|
| `apps/macafond` | Background daemon: audio, tray, hotkeys |
| `apps/macafon-cli` | `macafon` command line |
| `apps/macafon-ui` | Svelte 5 + Tauri v2 UI |
| `crates/macafon-dsp`, `macafon-studio`, `macafon-engine` | DSP chain, noise suppression, audio engine |
| `crates/macafon-sounddeck` | Sound library, decoding, playback |
| `crates/macafon-stt`, `apps/macafon-stt` | Dictation engines and the on-demand speech worker |
| `crates/macafon-backend*` | Windows (WASAPI, display profiles) and Linux (PipeWire) backends |
| `crates/macafon-ipc`, `macafon-config`, `macafon-apps` | Protocol, settings, app rules |

## Privacy and license

Everything runs locally: no accounts or telemetry. The only network requests are the dictation model
downloads you start yourself (from Hugging Face); audio and dictated text never leave your computer.

[GPL-3.0-only](LICENSE); `macafon-dsp` and `macafon-studio` are MIT OR Apache-2.0. Noise suppression uses
[RNNoise](https://github.com/jneem/nnnoiseless) and [DeepFilterNet](https://github.com/Rikorose/DeepFilterNet).
Dictation uses [whisper.cpp](https://github.com/ggml-org/whisper.cpp) with OpenAI Whisper models and
[sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) with NVIDIA Parakeet TDT 0.6B v3 (CC-BY-4.0).
VB-CABLE is a separate product by [VB-Audio](https://vb-audio.com/) and is not bundled.

## Alternatives

Macafon is a free, open-source alternative to SteelSeries Sonar (microphone processing and a per-app
mixer), Voicemeeter (a virtual microphone and routing) and Soundpad (a soundboard) — in one lightweight
app.

*Not affiliated with SteelSeries, VB-Audio or Leppsoft; product names are trademarks of their owners.*
