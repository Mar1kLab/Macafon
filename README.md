# Macafon

Lightweight, open-source microphone processing and per-app audio mixer for Linux (PipeWire) and Windows, written in Rust.
Think SteelSeries Sonar — faster, lighter and cross-platform.

> Status: Linux prototype. The daemon runs the virtual microphone with the full eight-module
> chain and built-in presets, is controlled live over a local socket (`macafon ctl`) and saves
> its settings. Calibration, A/B recording and playback, device profiles, and a Tauri desktop
> client are available. M1 validation and Linux packaging are in progress.

## Linux prototype

Requires stable Rust, PipeWire ≥ 1.0, WirePlumber ≥ 0.5, a C compiler, pkg-config,
libclang and the PipeWire development headers. On Ubuntu:

```sh
sudo apt install build-essential pkg-config libclang-dev libpipewire-0.3-dev pipewire-bin
cargo build --release -p macafond -p macafon-cli
target/release/macafond devices
target/release/macafond run --input '<physical node.name>'   # first run; remembered afterwards
```

While the daemon runs:

```sh
target/release/macafon ctl status
target/release/macafon ctl preset broadcast      # natural, discord-clear, noisy-room, broadcast, warm-podcast, raw
target/release/macafon ctl mute                  # toggle; also: mute on|off, bypass on|off
target/release/macafon ctl gain -3
target/release/macafon ctl devices               # then: macafon ctl device '<node.name>'
target/release/macafon ctl meters --count 10
```

Select **Macafon Microphone** in the consuming app. The prototype does not change system
defaults. Settings live in `$XDG_CONFIG_HOME/macafon/config.toml`; `macafond run` flags
(`--preset`, `--engine raw|lite`, `--intensity`, `--gain`, `--mute`, `--bypass`) override it.
The daemon waits for PipeWire and recreates the microphone after server restarts.
Stop with Ctrl+C. The M0 graph must run at 48 kHz; other graph rates are reported as unsupported.

Measured in a private headless PipeWire 1.6.2 graph at quantum 128: **2.67 ms raw**,
**22.67 ms Lite** (intensity 0, aligned dry path). The Lite latency target in the PRD
is not met; this remains an M0 investigation. All other modules add no latency.

`macafond route <playback-stream-id> <sink-node.name>` confirms actual WirePlumber
links and restores the previous target on timeout. Streams with `node.dont-reconnect`
or `node.dont-move` are rejected; moving every application remains an M0 investigation.

Local checks:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 tools/tests/pipewire-smoke.py
python3 tools/tests/wireplumber-routing.py
python3 tools/tests/control-smoke.py
```

See [Linux M0 development and limitations](docs/development/linux-m0.md),
[the microphone chain](docs/development/microphone-chain.md) and
[the daemon, protocol and config](docs/development/daemon.md) (RU).

## Documents

- [Product Requirements Document](docs/PRD.md) (RU)
- [SteelSeries Sonar & competitors analysis](docs/research/sonar-analysis.md) (RU)
- [Design system](docs/design/design-system.md) (RU)
- [ADR-0004: Windows on the free VB-CABLE: virtual mic, volume-group channels](docs/adr/0004-windows-free-vb-cable.md) (RU)
- ADR-0001 (APO) and ADR-0002 (channels on VB-Audio cables) are superseded by ADR-0004

## Local Linux packages

With the PipeWire and WebKitGTK 4.1 development packages, Rust and pnpm installed:

```sh
pnpm -C apps/macafon-ui install --frozen-lockfile
python3 tools/package-linux.py
```

This builds deb, rpm and AppImage artifacts under `target/release/bundle/` without publishing
anything. AUR source and removal hooks are under `packaging/aur/`; the matching binary recipe
with the deb checksum is generated under `target/package-linux/aur/macafon-bin/`.
PipeWire ≥ 1.0 and WirePlumber ≥ 0.5 are required at runtime, including for AppImage.

`macafon ctl set-default --confirm` explicitly selects the virtual microphone as the system
default. Quit restores the previous choice unless the user has changed it since;
`macafon restore-default` also performs recovery without a running daemon.

The application is licensed under GPL-3.0-only; `macafon-dsp` and `macafon-studio` under
MIT OR Apache-2.0. Remaining M1 validation and measured limits are recorded in
[the release checklist](docs/qa/release-checklist.md).
