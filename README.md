<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/branding/macafon-wordmark-white.svg">
    <img alt="Macafon" src="assets/branding/macafon-wordmark-black.svg" width="320">
  </picture>
</p>

<p align="center">
  <b>Open-source microphone processing and per-app audio mixer for Windows 11. Linux support is experimental.</b>
</p>

<p align="center">
  <img alt="License: GPL-3.0-only" src="https://img.shields.io/badge/license-GPL--3.0--only-blue">
  <img alt="Rust 2024" src="https://img.shields.io/badge/rust-2024-orange">
  <img alt="Windows 11: working" src="https://img.shields.io/badge/Windows_11-working-success">
  <img alt="Linux: experimental" src="https://img.shields.io/badge/Linux-experimental-orange">
</p>

![Windows microphone screen](docs/images/microphone.png)

## What it does

Macafon is a native Rust application with four screens: **Microphone, Mixer, Soundpad and Settings**.
Windows 11 is the working platform; Linux (PipeWire) is experimental.

**Microphone processing.** Macafon takes your physical microphone, runs it through a chain —
Gain → High-pass → AI noise suppression → Expander → EQ → Compressor → De-esser → Limiter — and exposes
the result as a virtual microphone for Discord, OBS, Zoom or games. On Windows, select
**CABLE Output (VB-Audio Virtual Cable)** in those apps.

- Simple module controls, with detailed tuning under *Advanced*.
- Two-step auto-calibration (stay silent, read a sentence) sets gain, expander, high-pass and noise suppression.
- Record a few seconds and switch **Before ⇄ After** while you tune — every change is heard immediately.
- Noise suppression *Lite* (RNNoise) or *Studio* (DeepFilterNet 3), running locally on the CPU.
- Six built-in presets plus your own, per-microphone profiles, hot-plug and a fallback microphone.

**Per-app mixer.** Start with Main, Voice and Music; customize up to eight channels in total. Apps show up
automatically and can be dragged between channels; Macafon remembers where each app belongs. On Windows,
channels are volume groups over audio sessions: moving an app changes which fader controls it, not its
audio device. Channels and Master have no EQ or effects. Choose an output device or an output priority
list for automatic switching when devices connect or disconnect.

![Mixer screen](docs/images/mixer.png)

**Windows controls.** Master follows the actual Windows volume and mute in both directions. Every action
supports primary and secondary hotkeys, with separate left/right Ctrl and Alt. Volume shortcuts repeat
while held and use your chosen step. Push-to-talk and push-to-mute work with the window closed.
Assigned shortcuts appear beside the controls.

**Background controls.** Right-click the Windows tray icon for a quick panel with microphone, preset
and volume controls. A global hotkey opens the horizontal mixer overlay. The configurable hotkey popup
(OSD) shows volume, mute, bypass and preset changes; choose its style, position and active or primary
monitor in Settings. The daemon keeps processing with the main window closed. Autostart, CLI commands
and **Doctor** diagnostics for device, cable and audio setup checks are included.

**Optional DAC controls.** When an iBasso Jr. Macaron is connected, device settings expose hardware volume,
balance, digital filter, gain and output mode. Volume is read from the device and follows its buttons.

**Soundpad (Windows).** Play sounds into your voice chat and your headphones at the same time, with
separate output switches and levels. The library lives in your `Music\Macafon Sounds` folder; its
first-level subfolders are categories. Copy files there, use **Add files**, or drag and drop audio
anywhere on the window. MP3, WAV, OGG, FLAC, M4A and AAC are supported.

- Search, sort (newest first by default), favorites, and category filters.
- Per-sound volume and primary/secondary global hotkeys; rename, move and delete sounds, and manage categories.
- Play/stop, pause/resume, repeat, seek and random playback; one sound plays at a time.
- **Mute my voice while playing** fades your voice out while a sound plays into the microphone.
- The hotkey overlay has an expandable Soundpad panel with search and category filters. It starts
  collapsed, remembers your search and category, and lets Enter play the first match.

![Windows Soundpad screen](docs/images/soundpad.png)

<details>
<summary>Settings, tray quick panel and expanded hotkey overlay (Windows mock UI)</summary>

![Windows Settings screen](docs/images/settings.png)
![Tray quick panel](docs/images/tray-panel.png)
![Hotkey overlay with Soundpad search](docs/images/hotkey-overlay.png)

</details>

Screenshots show the current source in browser mock mode, with sample devices, apps and sounds.

## Status

**Windows 11: working.** The current source includes the M3 Windows implementation and Soundpad.
The latest published installer is **v0.1.0 (October 17, 2026)**; Soundpad and its overlay panel were added
after that release. Build from source to use those additions until a newer installer is published.

**Linux: experimental.** The PipeWire backend, virtual microphone and channel effects are implemented,
but Linux builds and desktop integration need further validation. Linux is not covered by the Windows
build workflow.

Latency depends on the microphone and VB-CABLE buffers. Measurements and remaining release criteria
are recorded in the [release checklist](docs/qa/release-checklist.md).

## Download for Windows 11

1. Open the [latest release](https://github.com/Mar1kLab/Macafon/releases/latest)
   (currently [v0.1.0](https://github.com/Mar1kLab/Macafon/releases/tag/v0.1.0)).
2. Download and run `Macafon_0.1.0_x64-setup.exe` from **Assets**. `SHA256SUMS.txt` contains its SHA-256 checksum.
3. For the virtual microphone, download the free (donationware) [VB-CABLE](https://vb-audio.com/Cable/)
   from VB-Audio. Extract it, run `VBCABLE_Setup_x64.exe` as administrator, then reboot. Macafon does
   not bundle this driver.
4. Open Macafon and follow first-run onboarding: choose your microphone, calibrate, listen to the A/B
   test, and set up the mixer and background operation. If the cable is missing, onboarding provides
   the installation step and resumes after reboot. Setup can be skipped and reopened from Settings.
5. In Discord, OBS, Zoom or your game, select **CABLE Output (VB-Audio Virtual Cable)** as the input.
   Keep Macafon running: the cable is silent when the daemon is stopped.

The per-user installer does not require administrator rights for Macafon itself; installing
the separate VB-CABLE driver does. The current installer is unsigned.

An upgrade stops the running daemon, replaces the binaries and starts it again, keeping settings and
autostart. Uninstall restores default audio devices and managed application volumes; explicit changes
to system Master volume remain yours.

## Platforms

| | Windows 11 | Linux (experimental) |
|---|---|---|
| Virtual microphone | Free VB-CABLE, installed separately | Created by Macafon in PipeWire |
| Mixer | Up to eight application volume groups over Windows audio sessions | Virtual channels with EQ and effects |
| Master | System output volume and mute | Master gain over Macafon channels |
| Channel effects | Unavailable in Windows sessions mode; the future extra-cable mode is disabled | EQ, noise suppression, compressor and limiter |
| Soundpad | Library, voice-chat and headphone playback, overlay panel | Unavailable |
| Requirements | VB-CABLE for microphone processing; WebView2 for the UI | PipeWire >= 1.0, WirePlumber >= 0.5 |

Windows architecture: [ADR-0004](docs/adr/0004-windows-free-vb-cable.md).

## Building

### Windows 11

Install stable Rust, Node.js 24, Python 3.12 or newer, and the
[Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/#windows)
(Microsoft C++ Build Tools with Desktop development with C++, and WebView2).
The workflow uses Rust 1.99.0 and pnpm 12.11.2.

```powershell
npm install --global corepack@0.35.0
corepack enable
corepack pnpm -C apps/macafon-ui install --frozen-lockfile
python tools/package-windows.py            # target/release/bundle/nsis/
python tools/package-windows.py --install  # build, then install or upgrade
```

The manual [Windows workflow](.github/workflows/windows-build.yml) runs the same packaging script on a
GitHub-hosted Windows runner and uploads the installer and checksum. It does not install the app or
VB-CABLE on the runner.

### Linux (experimental)

You need stable Rust, Node.js with pnpm, Python 3 and the platform dependencies:

```sh
# Ubuntu / Debian
sudo apt install build-essential pkg-config libclang-dev libpipewire-0.3-dev pipewire-bin \
                 libwebkit2gtk-4.1-dev libgtk-3-dev libdbus-1-dev
corepack pnpm -C apps/macafon-ui install --frozen-lockfile
python3 tools/package-linux.py     # deb, rpm and AppImage under target/release/bundle/
```

Or run from source:

```sh
cargo build --release -p macafond -p macafon-cli
target/release/macafond devices
target/release/macafond run --input '<physical node.name>'
corepack pnpm -C apps/macafon-ui tauri dev
```

Select **Macafon Microphone** in your apps. An experimental AUR recipe is under `packaging/aur/`.

## Command line

```sh
macafon ctl status
macafon ctl preset broadcast        # natural, discord-clear, noisy-room, broadcast, warm-podcast, raw
macafon ctl mute                    # toggle; also mute on|off, bypass on|off
macafon ctl mic mute toggle         # for desktop shortcuts
macafon ctl channel voice volume +5
macafon ctl channel master volume -5
macafon ctl apps move bin:firefox music
macafon ctl doctor
macafon ctl soundpad list
macafon ctl soundpad play "Memes/airhorn.mp3"  # use an id from list
macafon ctl soundpad stop
macafon ctl soundpad pause                   # pause/resume
macafon ctl soundpad random                  # optional category argument
```

## Development

The UI runs in a regular browser against a mock daemon — no audio stack needed:

```sh
corepack pnpm -C apps/macafon-ui install --frozen-lockfile
corepack pnpm -C apps/macafon-ui dev
```

Open `http://127.0.0.1:1420/?platform=windows` for the Windows mock UI,
`?platform=windows&screen=soundpad` for Soundpad, or
`?platform=windows&view=quick-panel&mode=overlay` for the hotkey overlay.
Use `corepack pnpm -C apps/macafon-ui tauri dev` for the native UI with the real daemon.

Checks remain targeted and local. GitHub Actions currently provides the manual Windows installer build:

```sh
cargo fmt
cargo clippy -p <crate> -- -D warnings
cargo test -p <crate>
pnpm -C apps/macafon-ui check
python3 tools/tests/control-smoke.py   # Linux: a private PipeWire graph, the user's audio is untouched
```

Start with [AGENTS.md](AGENTS.md) — the contributor guidelines, including the hard real-time rules for
audio code.

The Cargo workspace is organized as follows:

| Path | Purpose |
|---|---|
| `apps/macafond` | Background daemon, tray, hotkeys and control server |
| `apps/macafon-cli` | `macafon` launcher and CLI |
| `apps/macafon-ui` | Svelte 5 UI and Tauri v2 shell |
| `crates/macafon-dsp`, `macafon-studio`, `macafon-engine` | DSP, Studio noise suppression and audio engine |
| `crates/macafon-soundpad` | Sound library, decoding, caching and playback |
| `crates/macafon-backend*` | Shared backend API, Windows WASAPI and experimental PipeWire backends |
| `crates/macafon-ipc`, `macafon-config`, `macafon-apps` | Protocol/generated UI types, settings/presets and app rules |
| `tools`, `packaging`, `docs` | Local checks, installers and project documentation |

| Document | |
|---|---|
| [Product requirements](docs/PRD.md) | Requirements and IDs, parameter ranges, architecture |
| [Design system](docs/design/design-system.md) | Tokens, typography, components |
| [Architecture decisions](docs/adr/) | ADR-0003 Linux channels, ADR-0004 Windows on VB-CABLE |
| [Daemon, protocol and config](docs/development/daemon.md) | IPC methods, config, calibration, A/B |
| [Microphone chain](docs/development/microphone-chain.md) · [Mixer](docs/development/mixer.md) · [UI](docs/development/ui.md) | Implementation notes |
| [Windows port](docs/development/windows-m3.md) | WASAPI path, measurements, installer |

## Privacy and license

Audio processing and Soundpad playback run locally. No accounts, telemetry, cloud audio processing or
background network requests. Fonts and noise-suppression models are bundled. The PRD permits only an
opt-in update check; downloads and driver installation are initiated by the user.

The application is licensed under [GPL-3.0-only](LICENSE). The `macafon-dsp` and `macafon-studio`
crates are available under MIT OR Apache-2.0. Noise suppression uses
[nnnoiseless](https://github.com/jneem/nnnoiseless) (RNNoise) and
[DeepFilterNet](https://github.com/Rikorose/DeepFilterNet); VB-CABLE is a separate product by
[VB-Audio](https://vb-audio.com/) and is not distributed with Macafon.
