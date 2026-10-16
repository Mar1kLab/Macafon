# M1: local release checklist

State as of 2026-10-11. Onboarding, warnings and M1 controls were added to the UI and checked in a
browser against the mock daemon ([ui.md](../development/ui.md)); the Tauri window does not start on this
test machine.

| Check | Command / method | Result |
|---|---|---|
| Rust formatting and warnings | `cargo fmt --all --check`; `cargo clippy --workspace -- -D warnings` | Passed |
| Shared APIs and allocation-free RT | `cargo test --workspace` | Passed; DSP/engine use `assert_no_alloc` |
| TypeScript protocol | `cargo test -p macafon-ipc --features ts typescript`; `pnpm -C apps/macafon-ui check` | Passed |
| Audio regression | `cargo test -p macafon-engine --test golden` | Passed: speech + fan, keyboard, street; 3 presets |
| Devices/channels/profiles | `python3 tools/tests/m1-inputs-smoke.py` | Passed in a private graph |
| Default source and restore | `python3 tools/tests/m1-defaults-smoke.py` | Passed: Quit, crash, a late user choice, the separate restore command |
| A/B and privacy | `python3 tools/tests/m1-ab-smoke.py` | Passed: float WAV A/B, no overwriting, cleanup when the UI client closes |
| Audio graph / restart | `python3 tools/tests/pipewire-smoke.py` | Passed: Raw 2.67 ms, Lite 22.67 ms, mute, two server restarts |
| Control / calibration | `python3 tools/tests/control-smoke.py` | Passed: calibration, profiles, A/B, autostart, PipeWire disconnect, saving within 0.55 s |
| Application build | `python3 tools/package-linux.py --build-only` | Passed |
| Linux packages | `python3 tools/package-linux.py --bundle-only` | deb/rpm/AppImage built; the AppImage CLI, daemon and autostart from the original path checked. AppImage ≈100 MiB with system libraries |
| Studio, quantum 128 | `python3 tools/tests/microphone-soak.py --seconds 20 --preset noisy-room` | Passed: 7499 observations, 0 xruns, the engine stayed Studio; RSS 50.2 MiB, CPU 3.60% of one core |
| Callback budget, NFR-S3 | `python3 tools/tests/profiler-smoke.py`; Rust test `graph_budget_fallback_updates_settings_and_device_profile` | Native measurements and quantum received; three consecutive overruns >80% switch Studio → Lite and save the profile |
| One-hour soak, quantum 128 | `python3 tools/tests/microphone-soak.py` | Failed: a quiet run stopped at 90 s after 2 xruns (33,768 observations). A short 5 s run — 1874 observations, 0 xruns |
| Physical loopback (§15.3) | Reference machine, a physical loop compared with the direct input | Needs a test machine |
| Suspend/resume, physical hot-plug | USB/built-in/Bluetooth, PipeWire + WirePlumber | Needs a test machine |
| Listening / NS-004 | Voice, keyboard/mouse clicks, silence; Lite/Studio | Needs a subjective assessment |
| §10.5 matrix / US-1…US-10 | Discord/Vesktop, browsers, OBS, games, Flatpak | M1 scenarios need a manual run; channels belong to M2 |
| deb/rpm/AUR install / uninstall | Clean supported distributions, an active user session | Needs VMs; package hooks checked syntactically |

## Open limitations

- NFR-L2: Lite has 20 ms of algorithmic latency (adapter frame + RNNoise overlap),
  22.67 ms in a private graph instead of ≤13 ms. The latency in the protocol reflects reality.
- NFR-R6: the deb `Installed-Size` is about 58 MiB; the <30 MB target is not met.
- NFR-R1…R3: micro-benchmarks do not replace a CPU measurement on a Ryzen 5 3600 / i5-10400.
- NFR-R4: locally without audio capture Raw RSS ≈12 MiB, Studio RSS ≈49 MiB (peak ≈51 MiB). Natural
  under audio load for 90 s: RSS 13.6 MiB, CPU 4.86% of one core. The reference machine and A/B need a
  separate measurement.
- NFR-S3: the full callback is measured through the standard PipeWire Profiler on the control thread.
  The subscription exists only while Studio runs; disabling Studio or stopping the stream removes it.
  Repeated completions are not counted; skipped cycles or a driver change reset the overrun streak.
  If the Profiler extension/server module is unavailable, the DFN3 worker detector remains, but the full
  callback cannot be measured in such an environment.
- The one-hour soak during heavy concurrent compilation showed xruns; this does not confirm NFR-S5. A
  repeated run without compilation also recorded 2 xruns in 90 s; the maximum callback time was 1.35 ms
  against a 2.67 ms budget. On this test machine the realtime priority limit = 0; this does not allow
  attributing the failure to DSP alone and does not confirm NFR-S5 on the reference machine.

## Packages and licenses

The build is local, without CI or publishing. `target/release/bundle/` contains the artifacts;
`target/package-linux/aur/macafon-bin/` — a recipe with the SHA256 of a specific deb.
The source AUR recipe in `packaging/aur/macafon/` uses the release tag `v0.1.0`; publishing first needs
that tag and the corresponding GitHub release assets.

The application is GPL-3.0-only; `macafon-dsp` and `macafon-studio` are MIT OR Apache-2.0.
The license texts of dependencies and fonts are collected into the package resources. The DFN3 model is
embedded; nothing is downloaded from the network at runtime. The audio fixtures have their own
attribution next to the WAV files.

The AppImage requires an installed PipeWire ≥1.0 and its client library: linuxdeploy excludes
`libpipewire-0.3.so.0` from the bundle. On a machine without FUSE, startup was checked with
`--appimage-extract-and-run`, with locally unpacked PipeWire/GTK dependencies.

On a repeated `tauri bundle` the bundler warns about the `__TAURI_BUNDLE_TYPE` marker in an already
patched binary. The packages were built and the CLI/daemon started; the updater is not used in M1.

## Additional budget and xrun measurements

A Studio run with native monitoring on this test machine stopped after 22 s: 2 xruns, 8251 observations,
a maximum completed callback of 1.89 ms, wake-up delays of 0.53 ms, 0 completed callbacks above 80% of the
budget. This does not prove the cause of the xruns: incomplete cycles and delays of other graph
participants need separate diagnostics. New soak reports store processing time, wake-up delay, deadline
misses and the number of budget overruns separately; a failing test also prints and saves its JSON.

In a separate comparison with A/B recording (10 s after warm-up) Studio CPU was 15.9% of one core without
the server Profiler and 16.1% with it. These local results do not confirm NFR-R3 <10%; the earlier 3.60%
measurement should not be taken as a guarantee.

The event format and completed/incomplete states were checked against the
[PipeWire 1.6.2 Profiler module](https://github.com/PipeWire/pipewire/blob/1.6.2/src/modules/module-profiler.c).

## M2 (in progress), 2026-10-11

| Check | Command / method | Result |
|---|---|---|
| Mixer end-to-end | `python3 tools/tests/mixer-smoke.py` | Passed in a private PipeWire 1.6.2 + WirePlumber 0.5.13 |
| Tray, BG-001/003 | `python3 tools/tests/tray-smoke.py` | Passed |
| Hotkeys (portal) | `python3 tools/tests/hotkeys-smoke.py` | Passed with a fake portal |
| Application volume (CH-008), reset when the mixer is disabled | `python3 tools/tests/mixer-smoke.py` | Passed |
| Default microphone (MIC-012), `devices.changed` (MIC-002) | `m1-defaults-smoke.py`, `m1-inputs-smoke.py` | Passed |
| Buffer size, close → quit, Doctor xruns/DSP load | `python3 tools/tests/control-smoke.py` | Passed (3 of 4 runs; once the independent "stereo idle meters" check failed — meter timing right after start) |
| Application icons on a real DE (APP-003) | Manual run | Needs a test machine; only htop/btop/vim/snap were checked on this one |
| Real KDE/GNOME/Hyprland portals, X11 grabs | A machine with a DE | Needs a test machine |
| Discord/Spotify/Flatpak/Electron in channels (§14.4) | Manual run | Needs a test machine |

## Windows (M3), 2026-10-16

Test machine: Windows 11 25H2, HyperX SoloCast, VB-CABLE 3.3.1.7. Details —
[windows-m3.md](../development/windows-m3.md).

| Check | Command / method | Result |
|---|---|---|
| Rust | `cargo clippy -p macafond -p macafon-ipc -p macafon-backend-windows -p macafon-ui --all-targets -- -D warnings`; `cargo test` of the same crates | Passed |
| UI | `corepack pnpm -C apps/macafon-ui check`; `vitest run` | Passed |
| Installer | `python tools/package-windows.py` | NSIS built, 16 MiB |
| Install without UAC / launch / uninstall | `setup.exe /S`, `macafon-ui.exe`, `uninstall.exe /S` | Passed; uninstall runs `macafond cleanup` |
| Uninstaller stops on a failed audio restore | `python tools/tests/windows-uninstall-hook.py` | Passed (fake daemon) |
| Onboarding on a clean system: install VB-CABLE → reboot → audio in Discord | Manual run | Needs a clean VM |
| Windows 10 22H2, `IPolicyConfig` | Manual run | Needs a test machine |
| Latency NFR-L6 ≤ 20 ms | `spike probe` with a reduced Max Latency | Not met: 82–138 ms with the default VB-CABLE buffer |
| One-hour soak | `spike run --seconds 3600` | Not run |
| §19.1 matrix: USB/Realtek/Bluetooth; Discord, OBS, Zoom, Teams, Telegram, games with anti-cheats | Manual run | Not run |
| Authenticode signing (SignPath) | — | Not configured |
