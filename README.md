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

**A clean voice in every app.** Macafon takes your physical microphone, runs it through a studio chain —
Gain → High-pass → AI noise suppression → Expander → EQ → Compressor → De-esser → Limiter — and exposes
the result as a virtual microphone for Discord, OBS, Zoom or games. On Windows, select
**CABLE Output (VB-Audio Virtual Cable)** in those apps.

- One slider per module, with every parameter one click away under *Advanced*.
- Two-step auto-calibration (stay silent, read a sentence) sets gain, noise gate and noise suppression.
- Record a few seconds and switch **Before ⇄ After** while you tune — every change is heard immediately.
- Noise suppression *Lite* (RNNoise) or *Studio* (DeepFilterNet 3), running locally on the CPU.
- Six built-in presets plus your own, per-microphone profiles, hot-plug and a fallback microphone.

**Sound split into channels.** Start with Main, Voice and Music; customize up to eight channels in total. Apps show up automatically
and are dragged between channels; Macafon remembers where each app belongs.

![Mixer screen](docs/images/mixer.png)

**Windows controls.** Master follows the actual Windows volume and mute in both directions. Every action
supports primary and secondary hotkeys, with separate left/right Ctrl and Alt. Volume shortcuts repeat
while held and use your chosen step. Assigned shortcuts appear beside the controls.

**Always within reach.** A tray panel, a hotkey mixer overlay and volume popups on the active or primary
monitor. The daemon keeps processing with the main window closed; on Windows the hidden panel stays
ready in the background. Autostart, CLI commands and a *Doctor* for setup checks are included.

**Optional DAC controls.** When an iBasso Jr. Macaron is connected, device settings expose hardware volume,
balance, digital filter, gain and output mode. Volume is read from the device and follows its buttons.

Audio processing runs locally. No accounts, cloud audio processing or telemetry.

## Status

**Windows 11: working.** The microphone chain, calibration, Before/After recording, application mixer,
system Master, tray, hotkeys, overlays, Macaron controls and installer are implemented and tested locally
on Windows 11. The project is currently version **0.1.0**; the available Windows features are functional.

**Linux: experimental.** The PipeWire backend, virtual microphone and channel effects are implemented,
but Linux builds and desktop integration need further validation. Linux is not covered by the Windows
build workflow.

Latency depends on the microphone and VB-CABLE buffers. Measurements and remaining release criteria
are recorded in the [release checklist](docs/qa/release-checklist.md).

## Download for Windows 11

1. Open the [latest release](https://github.com/Mar1kLab/Macafon/releases/latest).
2. Download and run `Macafon_0.1.0_x64-setup.exe` from **Assets**. `SHA256SUMS.txt` contains its SHA-256 checksum.
3. For the virtual microphone, install the free [VB-CABLE](https://vb-audio.com/Cable/) separately,
   then select **CABLE Output (VB-Audio Virtual Cable)** in Discord, OBS, Zoom or your game.

GitHub requires sign-in and repository access while this repository is private. Builds are also available
from [Windows 11 build](https://github.com/Mar1kLab/Macafon/actions/workflows/windows-build.yml) as the
**Macafon-Windows11-x64** artifact, retained for 30 days; release downloads have no artifact expiry.
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
```

## Development

The UI runs in a regular browser against a mock daemon — no audio stack needed:

```sh
pnpm -C apps/macafon-ui dev         # http://127.0.0.1:1420, add ?platform=windows for the Windows UI
```

Checks remain targeted and local. GitHub Actions currently provides the manual Windows installer build:

```sh
cargo fmt
cargo clippy -p <crate> --all-targets -- -D warnings
cargo test -p <crate>
pnpm -C apps/macafon-ui check
python3 tools/tests/control-smoke.py   # Linux: a private PipeWire graph, the user's audio is untouched
```

Start with [AGENTS.md](AGENTS.md) — the contributor guidelines, including the hard real-time rules for
audio code.

| Document | |
|---|---|
| [Product requirements](docs/PRD.md) | Requirements and IDs, parameter ranges, architecture |
| [Design system](docs/design/design-system.md) | Tokens, typography, components |
| [Architecture decisions](docs/adr/) | ADR-0003 Linux channels, ADR-0004 Windows on VB-CABLE |
| [Daemon, protocol and config](docs/development/daemon.md) | IPC methods, config, calibration, A/B |
| [Microphone chain](docs/development/microphone-chain.md) · [Mixer](docs/development/mixer.md) · [UI](docs/development/ui.md) | Implementation notes |
| [Windows port](docs/development/windows-m3.md) | WASAPI path, measurements, installer |

## License

The application is licensed under [GPL-3.0-only](LICENSE). The `macafon-dsp` and `macafon-studio`
crates are available under MIT OR Apache-2.0. Noise suppression uses
[nnnoiseless](https://github.com/jneem/nnnoiseless) (RNNoise) and
[DeepFilterNet](https://github.com/Rikorose/DeepFilterNet); VB-CABLE is a separate product by
[VB-Audio](https://vb-audio.com/) and is not distributed with Macafon.
