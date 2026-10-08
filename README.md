# micEQ

Lightweight, open-source microphone processing and per-app audio mixer for Linux (PipeWire) and Windows, written in Rust.
Think SteelSeries Sonar — faster, lighter and cross-platform.

> Status: Linux prototype. The daemon runs the virtual microphone with the full eight-module
> chain and built-in presets, is controlled live over a local socket (`miceq ctl`) and saves
> its settings. The M1 desktop app is not implemented yet.

## Linux prototype

Requires stable Rust, PipeWire ≥ 1.0, WirePlumber ≥ 0.5, a C compiler, pkg-config,
libclang and the PipeWire development headers. On Ubuntu:

```sh
sudo apt install build-essential pkg-config libclang-dev libpipewire-0.3-dev pipewire-bin
cargo build --release -p miceqd -p miceq-cli
target/release/miceqd devices
target/release/miceqd run --input '<physical node.name>'   # first run; remembered afterwards
```

While the daemon runs:

```sh
target/release/miceq ctl status
target/release/miceq ctl preset broadcast      # natural, discord-clear, noisy-room, broadcast, warm-podcast, raw
target/release/miceq ctl mute                  # toggle; also: mute on|off, bypass on|off
target/release/miceq ctl gain -3
target/release/miceq ctl devices               # then: miceq ctl device '<node.name>'
target/release/miceq ctl meters --count 10
```

Select **micEQ Microphone** in the consuming app. The prototype does not change system
defaults. Settings live in `$XDG_CONFIG_HOME/miceq/config.toml`; `miceqd run` flags
(`--preset`, `--engine raw|lite`, `--intensity`, `--gain`, `--mute`, `--bypass`) override it.
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
python3 tools/tests/control-smoke.py
```

See [Linux M0 development and limitations](docs/development/linux-m0.md),
[the microphone chain](docs/development/microphone-chain.md) and
[the daemon, protocol and config](docs/development/daemon.md) (RU).

## Documents

- [Product Requirements Document](docs/PRD.md) (RU)
- [SteelSeries Sonar & competitors analysis](docs/research/sonar-analysis.md) (RU)
- [Design system](docs/design/design-system.md) (RU)
- [ADR-0001: Windows mic processing via APO](docs/adr/0001-windows-apo.md) (RU)
- [ADR-0002: Windows output channels via user-installed VB-Cable](docs/adr/0002-windows-output-vbcable.md) (RU)
