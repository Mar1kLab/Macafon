# micEQ

Lightweight, open-source microphone processing and per-app audio mixer for Linux (PipeWire) and Windows, written in Rust.
Think SteelSeries Sonar — faster, lighter and cross-platform.

> Status: Linux prototype. Virtual microphone with the full eight-module processing chain and
> built-in presets works from the CLI; the M1 desktop app, IPC and config are not implemented yet.

## Linux prototype

Requires stable Rust, PipeWire ≥ 1.0, WirePlumber ≥ 0.5, a C compiler, pkg-config,
libclang and the PipeWire development headers. On Ubuntu:

```sh
sudo apt install build-essential pkg-config libclang-dev libpipewire-0.3-dev pipewire-bin
cargo build --release -p miceqd
target/release/miceqd devices
target/release/miceqd run --input '<physical node.name>'                 # Natural preset
target/release/miceqd run --input '<physical node.name>' --preset broadcast
```

Select **micEQ Microphone** in the consuming app. The prototype does not change system
defaults. Presets: `natural`, `discord-clear`, `noisy-room`, `broadcast`, `warm-podcast`, `raw`.
`--engine raw|lite`, `--intensity`, `--gain` override the preset; `--bypass` passes the
microphone through unprocessed; `--mute` outputs digital silence.
The daemon waits for PipeWire and recreates the microphone after server restarts.
Stop with Ctrl+C. The M0 graph must run at 48 kHz; other graph rates are reported as unsupported.

Measured in a private headless PipeWire 1.6.2 graph at quantum 128: **2.67 ms raw**,
**22.67 ms Lite** (intensity 0, aligned dry path). The Lite latency target in the PRD
is not met; this remains an M0 investigation. All other modules add no latency.

`miceqd route <playback-stream-id> <sink-node.name>` confirms actual WirePlumber
links and restores the previous target on timeout. Streams with `node.dont-reconnect`
or `node.dont-move` are rejected; moving every application remains an M0 investigation.

Local checks:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 tools/tests/pipewire-smoke.py
python3 tools/tests/wireplumber-routing.py
```

See [Linux M0 development and limitations](docs/development/linux-m0.md) and
[the microphone chain](docs/development/microphone-chain.md) (RU).

## Documents

- [Product Requirements Document](docs/PRD.md) (RU)
- [SteelSeries Sonar & competitors analysis](docs/research/sonar-analysis.md) (RU)
- [Design system](docs/design/design-system.md) (RU)
- [ADR-0001: Windows mic processing via APO](docs/adr/0001-windows-apo.md) (RU)
- [ADR-0002: Windows output channels via user-installed VB-Cable](docs/adr/0002-windows-output-vbcable.md) (RU)
