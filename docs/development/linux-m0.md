# Linux M0: the first working audio path

State as of 2026-10-09: M0 from PRD §5/§19 has started. This is a technical prototype without UI, not
the finished M1 MVP. The final choice of design and the ADR are postponed until the latency,
DeepFilterNet and PipeWire/WirePlumber version compatibility checks are complete.

## What is implemented

- A Cargo workspace, stable Rust, edition 2024. Only the crates used now were created —
  `macafon-dsp`, `macafon-backend-pipewire`, `macafond`; the rest will appear together with the code.
- One `pw_filter` of our own, `Audio/Source/Virtual`, `macafon_mic`, description
  `Macafon Microphone` — the basis of MIC-010/MIC-011. Mono float32 input/output, 48 kHz graph.
- A raw path with zero algorithmic latency, digital mute (MIC-013), NaN/Inf/denormal sanitization at
  the chain boundary (NFR-S4).
- RNNoise Lite through `nnnoiseless`, a 480-sample frame adapter for any quantum, dry/wet alignment and
  10 ms smoothing of the VAD attenuation (the basis of NS-001/DSP-003). Intensity/VAD ranges and
  defaults are centralized in `macafon-dsp`.
- Memory is allocated before the filter connects. The whole callback is checked with `assert_no_alloc`,
  including in the release build. PipeWire enables FTZ/DAZ through the `cpu.zero.denormals` context
  property (NFR-S1/NFR-S4).
- Input binding by `node.name`, an explicit mono/left port. The registry tracks ports disappearing and
  returning: the source stays, outputs silence without an input, and the link is recreated when the
  same name returns (the basis of MIC-005).
- A CLI list of devices/streams with `node.description`. Our own nodes and `.monitor` are excluded (the
  basis of MIC-001/MIC-003; no icons or UI yet).
- `route` checks stream/sink types, writes `target.object` into the `default` metadata by the sink's
  `object.serial` and confirms the move by real WirePlumber links (the basis of APP-004). On timeout it
  restores the previous target if another client has not changed it meanwhile.
  `node.dont-reconnect`/`node.dont-move` are checked through the full NodeInfo: the short registry
  properties do not contain these flags.
- After losing PipeWire the daemon waits for the server and recreates the filter/links/DSP outside RT,
  keeping the selected input and CLI parameters (the basis of NFR-S8). SIGINT/SIGTERM work while
  waiting; two consecutive restarts are checked by an integration test.
- The fixed DSP ports come with a node `EnumFormat` mono F32P/48 kHz and publication of the negotiated
  `Format`/`Props` on the control loop. `pw_filter` has no node `PortConfig`; WirePlumber finishes
  negotiating the existing ports after a `Props` notification with a mono channelMap. This lets
  WirePlumber discover the source and connect an ordinary consumer (MIC-010/MIC-011) without changing
  ports or DSP state in RT.
- The algorithmic latency is reported through SPA `ProcessLatency` (NFR-L4). SIGINT/SIGTERM are handled
  before the data-loop threads start; exiting frees the node.

`macafond run` installs nothing into systemd and does not change system default devices. Routing changes
only on an explicit `route` command; rules are not persisted yet. One filter instance corresponds to one
microphone; DSP state is not global, so future independent chain instances will not require DSP rework.

## Build and run

Requires PipeWire ≥ 1.0, WirePlumber ≥ 0.5, stable Rust, a C compiler, pkg-config, libclang and the
PipeWire/SPA headers. On Ubuntu:

```sh
sudo apt install build-essential pkg-config libclang-dev libpipewire-0.3-dev pipewire-bin
cargo build --release -p macafond
target/release/macafond devices
target/release/macafond run --input 'alsa_input.<name from devices>' --preset raw --engine lite
```

Use a release build for real time: debug inference and a concurrent build can cause skipped cycles and
unstable measurements on a headless machine. Select `Macafon Microphone` in the consuming application.
Ctrl+C ends the prototype. Without `--input` the input stays unconnected; for experiments it can be
connected with `pw-link`. If the given device is missing, the prototype waits for that name to appear.
If the server is not running yet or has restarted, it retries the connection every 500 ms. The RNNoise
state is recreated on recovery; a short audio gap is unavoidable.

```sh
target/release/macafond run --input 'alsa_input.<name>' --preset raw
target/release/macafond run --input 'alsa_input.<name>' --preset raw --engine lite --intensity 70 --voice-threshold 50
target/release/macafond run --input 'alsa_input.<name>' --mute
target/release/macafond route <playback-stream-id> <sink-node.name>
```

`devices` prints the ID, media.class, node.name and a human-readable description. `route` waits up to
three seconds for the actual move to be confirmed by active/ready links. A policy error is not counted
as a successful metadata write. WirePlumber must allow `linking.allow-moving-streams`; the command rejects
streams with `node.dont-reconnect=true` or `node.dont-move=true` without changing their target. This is a
confirmed limitation of the APP-004 approach, not a closed "move any application" requirement. There is
no automatic takeover of applications and no persistence yet.

## Local checks

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release -p macafond -p macafon-cli
python3 tools/tests/pipewire-smoke.py
python3 tools/tests/wireplumber-routing.py
cargo bench -p macafon-dsp --bench microphone -- --sample-size 10 --measurement-time 1 --warm-up-time 1
```

The smoke test uses a separate PipeWire with a private runtime directory and a synthetic microphone. The
user's audio server and physical devices are not touched. It needs `pipewire`, `pw-cat`, `pw-link`,
`pw-dump`, `pw-metadata` from the PipeWire tools package. The test ends its child processes and removes the
temporary runtime directory itself. Snapshots of the synthetic signal stay in `.artifacts/latency/`,
excluded from git.

It checks signal flow, mute, silence after the input disappears, reconnection by name, removal of the
source on SIGTERM, two recoveries after a server loss with Lite/latency preserved and signal flowing,
waiting for a missing server with exit on SIGINT, a routing error without a policy manager and a metadata
rollback. A DSP test separately runs the first block on a new thread under `assert_no_alloc` and checks
arbitrary block sizes, dry path alignment, NaN/Inf and a bit-exact raw path.

The separate `wireplumber-routing.py` requires WirePlumber, `wpctl` and `dbus-run-session`. It creates a
private D-Bus/runtime/config/state, starts only the policy profile without hardware monitors and does not
touch the user's WirePlumber. It checks Main → Voice → Main → Voice without recreating the stream, refusal
with `linking.allow-moving-streams` disabled with the previous target restored, an explicit refusal for
`node.dont-reconnect=true`/`node.dont-move=true`, as well as a consumer connecting to the virtual
microphone through WirePlumber with audio flowing, and the A/B stream connecting automatically to the
selected output and to the default output.

`tools/latency-probe/analyze.py` compares synchronous channels of stereo float32 LE raw:
FL — the direct path of a synthetic impulse, FR — through Macafon. An unstable latency is treated as an
error, not averaged. Example:

```sh
python3 tools/latency-probe/analyze.py .artifacts/latency/raw.raw
```

## M0 result and open problems

On PipeWire 1.6.2, 48 kHz, quantum 128, release:

| Path | Algorithmic latency | Measured added latency |
|---|---|---|
| Raw | 0 samples | 128 samples / 2.67 ms — NFR-L1 met in a headless graph |
| Lite, intensity 0 | 960 samples / 20 ms | 1088 samples / 22.67 ms — NFR-L2 **not met** |

Intensity 0 keeps RNNoise enabled and running but uses the aligned dry signal for an exact impulse
measurement. `--engine raw` is used for a full bypass without latency. This measurement does not replace
a check of the wet path on speech and a physical loopback.

PRD §10.1 takes the Lite latency as 10 ms. `nnnoiseless` uses a two-frame overlap-add window; the
streaming adapter additionally collects a frame. The current prototype ends up with two frames, 20 ms,
plus one graph quantum. The PRD target was not changed: reducing the adapter latency / graph design needs
investigation, and then a decision has to be agreed.

`nnnoiseless` is pinned to **0.5.1**: in 0.5.2 the FFT moved to `easyfft` with lazy thread-local
planners/buffers. Preparing in the control thread does not prevent the first allocation on another RT
thread. Version 0.5.1 creates the FFT and scratch at construction and passes the first-block check on a
new thread. The BSD-3-Clause license is compatible with the license options from PRD Q1; this round did
not choose the project's own license.

Remaining before M0 is complete:

- DeepFilterNet/Studio: the model, an RT safety check and the CPU/latency budget (NS-002).
- A PipeWire/WirePlumber version matrix and real native/Flatpak/Electron applications. On PipeWire 1.6.2 /
  WirePlumber 0.5.13 an ordinary native stream is moved; `node.dont-reconnect=true` blocks the move. An
  approach for such streams still has to be chosen without violating the user's policy/settings.
- Suspend/resume and physical device reconnection (NFR-S8/MIC-005): synthetic hot-plug and a server
  restart are checked, hardware scenarios are not yet.
- A physical loopback and a one-hour soak counting xruns, a CPU/RAM check on the reference PC.
- Support for 44.1/96 kHz graphs through the PipeWire adapter (DSP-006): currently a rate mismatch leads
  to silence and a diagnostic exit rather than processing at the wrong rate.

The eight-module chain and the M1 presets are described in [microphone-chain.md](microphone-chain.md);
the latency measurements above were made with `--preset raw`. UI, IPC, config, calibration/A-B, tray,
autostart and packaging belong to M1 and are not done yet.


## Experimental Linux dictation

Dictation now uses the shared STT worker on Linux (PRD §8.4.9, ADR-0005). Build both binaries
with `WHISPER_DONT_GENERATE_BINDINGS=1 cargo build --release -p macafond -p macafon-stt-worker` and keep `macafon-stt` beside
`macafond`. Native worker builds need CMake, a C/C++ toolchain and libclang; the sherpa-onnx
build fetches its upstream Linux runtime archive. Ship its shared libraries beside the worker,
including ONNX Runtime; the Linux worker searches its own directory through `$ORIGIN`
without changing the user's loader environment. Linux defaults
to CPU inference without a Vulkan SDK; the Windows feature set is unchanged.
`tools/package-linux.py` sets `WHISPER_DONT_GENERATE_BINDINGS=1` for its build
subprocesses, using whisper-rs-sys 0.14.1's bundled CPU bindings: its bindgen version
misreads C struct layouts with libclang 23. Use the same prefix for direct Linux
Cargo builds. Do not export this globally or set it in `.cargo/config.toml`:
Windows Vulkan builds require generated bindings for their additional symbols.

In the Dictation screen, download a model explicitly, enable dictation, and assign Hold and/or
Hands-free in Hotkeys. Use a key-bearing shortcut such as `Ctrl+Super+F9`; pure modifier chords
are Windows-specific. Portal press/release signals and X11 grabs share the daemon's hold logic.
Wayland requires `wtype` (if supported by the compositor) and `wl-clipboard`; X11 uses `xdotool`
and `xclip` or `xsel`. Optional `ydotool` can inject Ctrl+V through an existing accessible socket;
Macafon does not set up ydotoold or request root. Missing/blocked injection falls back to copying;
without a clipboard utility, it reports an insertion error. Fixed and Auto are the supported
language policies. No game detection is available, so select copy-only dictation for games.

Storage follows XDG data paths. Paste restores plain text after approximately 300 ms if the
clipboard still contains the dictated text; rich formats/history exclusion and identical-text
change detection are unavailable. Wayland inserts into the focus at completion; X11 checks the
window captured at key press. Lightweight replaceable desktop notifications show dictation
states without recognized text. Custom popup positioning and looks are Windows-only.

Browser development remains `pnpm -C apps/macafon-ui dev`; open
`?screen=dictation&dictation=ready` for the Linux fixture and add `platform=windows` to compare.
Desktop utilities are optional build dependencies; command selection, focus fallback and
clipboard restore decisions are unit tested without a desktop session.

## Display profiles

The Linux-only `macafon-displays-linux` crate implements §8.5 / §9.10 independently of PipeWire.
Launch the daemon inside the graphical user's session, with its D-Bus, Wayland/X11 environment
and the relevant CLI in `PATH`. No privileged helper, installed driver or additional Rust
runtime dependency is required: zbus 5, serde_json and thiserror are already used in the workspace.
The daemon advertises `hello.capabilities.displays` only after a real inventory query succeeds.
Detection is cached for the run; restart after installing a helper or changing desktops.

| Desktop | Backend | Optional fields / limits |
|---|---|---|
| GNOME, X11 or Wayland | `org.gnome.Mutter.DisplayConfig` over the session bus | Primary, logical-monitor mirrors and HDR color mode where advertised. Verify 0 → temporary 1 → persistent 2 on Keep. GNOME Shell can ask for a second confirmation before saving to disk. |
| Plasma, X11 or Wayland | `kscreen-doctor -j` | Primary and HDR where advertised; mirror control only when `--help` advertises `.mirror.`. No CLI dry-run: validate connected modes and topology locally, then rely on rollback if the compositor rejects the apply. |
| Compatible wlroots compositor | `wlr-randr --json` | Resolution/refresh, position, rotation, enable; `--dryrun` before apply. No primary/mirror/HDR. Sway works only with the output-management protocol exposed; Hyprland versions without that protocol are unavailable. |
| Native X11 fallback | `xrandr --verbose` | EDID, physical mode/refresh, primary, rotation, clone/extend, enable; `--dryrun` before apply. HDR ignored. Never selected for Xwayland on a Wayland session. |

Raw DRM EDID is matched only by an unambiguous connector (including HDMI-A/HDMI name differences).
Without raw EDID, Mutter's or wlroots' reported identity is used; KScreen/XRandR can fall back to
connector-only identity. Identical monitors without serials retain the ADR-0006 port ambiguity.
Scaling is preserved where reported and remains outside profile editing. Reflections, custom
color profiles, VRR and compositor-specific features are outside the profile contract. CLI apply
persistence across logins depends on the desktop. Display inventory polling is every 2 s for CLI
backends; Mutter uses `MonitorsChanged`.

`macafon ctl display` uses the existing IPC commands. The ksni tray has Display profile, All monitors
on and Previous layout. All monitors on uses preferred modes and stays temporary on Mutter to
avoid a second compositor confirmation in the emergency path. Safety shortcuts are offered once on Linux (defaults revision 6), preserving
custom/cleared bindings and avoiding conflicts. Unsupported sessions hide the actions. Notification
Keep / Revert actions are optional: the daemon always owns the 15 s auto-revert deadline and keeps
the recovery snapshot if restore fails. The browser mock and existing UI need no Linux-specific
components or duplicated protocol types.

Targeted local checks:

```sh
cargo fmt
cargo clippy -p macafon-displays-linux -p macafond -p macafon-cli --all-targets -- -D warnings
cargo test -p macafon-displays-linux
cargo test -p macafond displays
cargo test -p macafon-config display_defaults
```

The parser fixtures, identity resolution, mirrored command plans, all-off rejection and Mutter
variant round trip run without a GUI. Live apply/Keep/timeout/revert,
hotplug, notification actions, tray rendering, HDR and persistence still need a desktop hardware
check. No display e2e, cloud workflow or Windows build is added.

## Linux feature packages

The historical M0 notes above describe the initial prototype. The current Linux package also ships
SoundDeck (§7.6), dictation (§8.4) and display profiles (§8.5), with desktop support depending on the
session/compositor. Build locally with stable Rust, C/C++, CMake, libclang, pkg-config, patchelf,
Node/pnpm and the PipeWire, GTK 3, WebKitGTK 4.1 development packages:

```sh
pnpm -C apps/macafon-ui install --frozen-lockfile
python3 tools/package-linux.py --build-only
python3 tools/package-linux.py --bundle-only --bundles deb,rpm,appimage
```

Use the distribution's development packages. `--build-only` builds the
UI, daemon, CLI and `macafon-stt-worker` (binary `macafon-stt`), then stages the worker under
`target/package-linux/runtime/usr/bin/` and inference libraries under `runtime/usr/lib/macafon/`.
The worker's `$ORIGIN/../lib/macafon:$ORIGIN` runpath and the libraries' `$ORIGIN` runpaths allow relocation;
packaging fails if `ldd` finds an unresolved dependency. Do not copy only the worker executable.
`patchelf` changes staged files only. Tauri also copies the staged worker into `target/release`;
the second runpath entry lets that development copy find the adjacent Cargo runtime libraries. A source
AUR build installs this same staged runtime tree. AppImage also carries the private tree;
linuxdeploy may additionally place its dependency copies in `usr/lib` and rewrite the worker's runpath
to `$ORIGIN/../lib`. Both layouts use relative paths inside the image.
No `LD_LIBRARY_PATH` modification or separate systemd worker service is needed.

Linux builds enable Whisper and Parakeet CPU inference. The Vulkan build was tried on the prepared
host and failed because Vulkan headers/library and `glslc` were unavailable; Linux target dependency
selection therefore omits `vulkan`. Windows retains its existing Vulkan features. This is a Linux
limitation of STT-020; a GPU selection cannot provide GPU acceleration with this worker.

Artifacts are in `target/release/bundle/`. Install the deb/rpm through the distribution package manager,
or make the AppImage executable and run it (`--appimage-extract-and-run` if FUSE is unavailable).
Launch `macafon-ui`, or run `macafond run` and `macafon ctl status` for diagnostics. Installed packages
include the systemd **user** unit; start it with `systemctl --user start macafond` only when no daemon is
already running. Autostart stays opt-in. Deb/rpm post-install hooks reload logged-in
users' systemd managers and restart only an already active `macafond.service`, so
upgrades use the new daemon and worker without enabling a stopped service.
Logged-out users load the new files on their next login. Dictation launches the adjacent worker on
demand. For a direct staged loader check:

```sh
env -u LD_LIBRARY_PATH target/package-linux/runtime/usr/bin/macafon-stt --help
readelf -d target/package-linux/runtime/usr/bin/macafon-stt
```

Models are not in the package: import an existing verified local model or explicitly download one in
Dictation. SoundDeck defaults to `Macafon Sounds` inside `XDG_MUSIC_DIR`, read from
`$XDG_CONFIG_HOME/user-dirs.dirs` (normally `~/.config/user-dirs.dirs`), with `~/Music` as the fallback.
An explicit SoundDeck folder setting continues to take precedence.

Optional helpers (install only those appropriate for the desktop):

- Dictation insertion: `wtype` for compatible Wayland compositors, `xdotool` for X11, or `ydotool`
  with its separately configured service and input-device permissions. Macafon does not elevate itself
  or configure that service.
- Clipboard: `wl-clipboard` on Wayland or `xclip` on X11.
- Displays: `kscreen-doctor` from `kscreen` on KDE, `wlr-randr` on compatible wlroots compositors,
  or `xrandr` (`xorg-xrandr` on Arch, `x11-xserver-utils` on Debian) on X11. Other compositors may not
  expose the protocols these helpers need.
- SoundDeck Open folder: `xdg-open` from `xdg-utils`.

These are AUR optdepends and deb/rpm recommendations, not hard dependencies. Packaging does not change
system audio/display settings or grant input-device permissions. Run the Linux feature checks in the
[release checklist](../qa/release-checklist.md#linux-feature-release-gate) on the target desktops.
