<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/branding/macafon-wordmark-white.svg">
    <img alt="Macafon" src="assets/branding/macafon-wordmark-black.svg" width="320">
  </picture>
</p>

<p align="center">
  <b>Lightweight, open-source microphone processing and per-app audio mixer for Linux and Windows.</b><br>
  Think SteelSeries Sonar — lighter, faster to set up, and not tied to one OS or one brand of headset.
</p>

<p align="center">
  <img alt="License: GPL-3.0-only" src="https://img.shields.io/badge/license-GPL--3.0--only-blue">
  <img alt="Rust 2024" src="https://img.shields.io/badge/rust-2024-orange">
  <img alt="Linux: PipeWire" src="https://img.shields.io/badge/linux-PipeWire-informational">
  <img alt="Windows: VB-CABLE" src="https://img.shields.io/badge/windows-VB--CABLE-informational">
</p>

![Microphone screen](docs/images/microphone.png)

## What it does

**A clean voice in every app.** Macafon takes your physical microphone, runs it through a studio chain —
Gain → High-pass → AI noise suppression → Expander → EQ → Compressor → De-esser → Limiter — and exposes
the result as a virtual microphone, **Macafon Microphone**, that you pick in Discord, OBS, Zoom or games.

- One slider per module, with every parameter one click away under *Advanced*.
- Two-step auto-calibration (stay silent, read a sentence) sets gain, noise gate and noise suppression.
- Record a few seconds and switch **Before ⇄ After** while you tune — every change is heard immediately.
- Noise suppression *Lite* (RNNoise) or *Studio* (DeepFilterNet 3), running locally on the CPU.
- Six built-in presets plus your own, per-microphone profiles, hot-plug and a fallback microphone.

**Sound split into channels.** Main, Voice, Music and up to eight of your own. Apps show up automatically
and are dragged between channels; Macafon remembers where each app belongs.

![Mixer screen](docs/images/mixer.png)

**Stays out of the way.** A small daemon runs in the tray and starts with your session; the window
unloads completely when closed. Global hotkeys (mute, push-to-talk, presets, channel volume), a CLI for
scripts, and a *Doctor* that checks the setup and fixes common problems in one click. No accounts, no
cloud, no telemetry.

## Platforms

| | Linux | Windows 10 22H2+ / 11 |
|---|---|---|
| Virtual microphone | Created by Macafon in PipeWire | The free [VB-CABLE](https://vb-audio.com/Cable/) driver, installed by you |
| Mic latency without noise suppression | ≈ 2.7 ms (one graph cycle) | Higher: two WASAPI passes plus the cable buffer |
| Mixer channels | Real virtual devices with per-channel EQ, noise suppression, compressor, limiter | Volume groups over Windows audio sessions, no per-channel effects |
| Requirements | PipeWire ≥ 1.0, WirePlumber ≥ 0.5 | VB-CABLE; no administrator rights for Macafon itself |

Why Windows works this way is explained in [ADR-0004](docs/adr/0004-windows-free-vb-cable.md).

## Status

Pre-release. The Linux microphone (M1) and mixer (M2) and the Windows port (M3) are implemented and
tested locally; there are no published releases yet. Known gaps and measured limits are tracked in the
[release checklist](docs/qa/release-checklist.md) — notably the Lite noise suppression latency (≈ 22.7 ms
against a 13 ms target) and the Windows microphone latency, which depends mostly on the VB-CABLE buffer.

## Building

You need stable Rust, Node.js with pnpm (Corepack is fine) and the platform prerequisites below.

### Linux

```sh
# Ubuntu / Debian
sudo apt install build-essential pkg-config libclang-dev libpipewire-0.3-dev pipewire-bin \
                 libwebkit2gtk-4.1-dev libgtk-3-dev libdbus-1-dev
pnpm -C apps/macafon-ui install --frozen-lockfile
python3 tools/package-linux.py      # deb, rpm and AppImage under target/release/bundle/
```

Or run from source without packaging:

```sh
cargo build --release -p macafond -p macafon-cli
target/release/macafond devices
target/release/macafond run --input '<physical node.name>'   # remembered afterwards
pnpm -C apps/macafon-ui tauri dev                           # the desktop UI
```

Then select **Macafon Microphone** in your apps. An AUR recipe is under `packaging/aur/`.

### Windows

```sh
python tools/package-windows.py     # per-user NSIS installer under target/release/bundle/nsis/
```

The installer needs no administrator rights and does not include VB-CABLE: onboarding links to the
official VB-Audio download page. Uninstalling gives back your default microphone, output and app volumes.

## Command line

```sh
macafon ctl status
macafon ctl preset broadcast        # natural, discord-clear, noisy-room, broadcast, warm-podcast, raw
macafon ctl mute                    # toggle; also mute on|off, bypass on|off
macafon ctl mic mute toggle         # for desktop shortcuts
macafon ctl channel voice volume +5
macafon ctl apps move bin:firefox music
macafon ctl doctor
```

## Development

The UI runs in a regular browser against a mock daemon — no audio stack needed:

```sh
pnpm -C apps/macafon-ui dev         # http://127.0.0.1:1420, add ?platform=windows for the Windows UI
```

Checks are targeted and local (there is no cloud CI):

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
| [Sonar analysis](docs/research/sonar-analysis.md) | Why the product looks the way it does |

## License

The application is licensed under [GPL-3.0-only](LICENSE). The `macafon-dsp` and `macafon-studio`
crates are available under MIT OR Apache-2.0. Noise suppression uses
[nnnoiseless](https://github.com/jneem/nnnoiseless) (RNNoise) and
[DeepFilterNet](https://github.com/Rikorose/DeepFilterNet); VB-CABLE is a separate product by
[VB-Audio](https://vb-audio.com/) and is not distributed with Macafon.
