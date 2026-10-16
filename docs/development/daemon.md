# M1: daemon, live control, config and IPC

State as of 2026-10-12. The `macafond` daemon runs continuously and can be controlled while it runs over
a Unix socket (a named pipe on Windows); settings are saved. UI — [ui.md](ui.md), scripts and hotkeys —
`macafon ctl`.

## Structure

`apps/macafond/src/host.rs` holds the daemon state and lifecycle; command implementations live in
`host/`: `microphone`, `capture` (calibration and A/B), `presets`, `mixer`, `hotkeys` and the shared
`requests` dispatcher. All of them run on the control loop.
In the PipeWire backend `connection` manages the connection, `nodes` — device discovery, `graph` — input
links, `session` — building and maintaining one session. The RT handlers stay in `filter`,
`channel_node`, `meter` and `player`.

`macafon-ipc` describes requests (`request`), state (`state`), events and replies (`wire`) separately; the
existing public type paths and the JSON protocol are preserved. `framing` reads lines with a single limit
shared by the server, the CLI client and the Tauri bridge: an unterminated or overlong line is rejected
before JSON deserialization. `macafon-config::storage` is responsible for reading, migration, recovery and
atomic writes; TOML is parsed once. In the CLI, arguments, command execution, launching sibling binaries
and output formatting are split into `args`, `control`, `launch` and `output`.

```
IPC threads (2 per client) ──pipewire::channel──► PipeWire control loop (DaemonHost)
        ▲   meters 1–60 Hz                                │ MicControl::apply
   Hub (event broadcast, meter pump)                      ▼ rtrb, allocation-free
                                                     MicRt in the pw_filter RT callback
```

- `macafon-engine`: `MicControl` (control loop) and `MicRt` (RT). Continuous changes go into an `rtrb`
  queue and are applied by `retune` with 10 ms smoothing. Structural changes (module on/off, bypass,
  look-ahead) build a new processor outside RT; `MicRt` crossfades over 10 ms, and the old processor is
  returned through a second queue and freed on the control loop (NFR-S2, DSP-003). Retune, replacement and
  queue overflow are checked under `assert_no_alloc`.
- The chain latency after changes is reported to the graph by updating `ProcessLatency` (NFR-L4).
- Changing the microphone applies without a restart: the link is recreated on the next tick (100 ms).
  Input disconnect/connect is broadcast as a `mic.changed` event (the basis of the MIC-005 toast).
- The fallback microphone (MIC-006) is set with `macafon ctl fallback-device '<node.name>'` (`none`
  disables the fallback). The primary always has priority; when it disconnects, an available fallback is
  selected, and when the primary returns the daemon switches back. If both are disconnected, the virtual
  microphone stays and outputs silence (MIC-005, MIC-010). `macafon ctl device none` disconnects the input
  entirely, including the fallback. `MicState.device` is the saved primary, `fallback_device` the fallback,
  `active_device` the currently connected input. Changing the active microphone interrupts calibration and
  A/B recording so that data from different devices is not mixed; the measured noise floor is reset. The
  chain stays shared — per-device profiles (MIC-007) are not done yet.
- Stereo mode of the virtual microphone (MIC-011): `macafond run --stereo` or `stereo = true` in the
  config's `[mic]`. FL and FR ports are created instead of MONO; DSP processes the mono signal once and
  copies the result into both channels. No extra latency. A consumer connected to only one of the channels
  works too. The input and A/B recording stay mono. `MicState.stereo` shows the output format. The format
  is applied at daemon start; to go back to mono — `--stereo=false` or `stereo = false` and a restart.
  When the output buffers are updated, the old ID in the PipeWire IO is reset: otherwise after hot-plug one
  buffer is queued twice and the channels read different audio blocks.
- Meters are computed in RT only while there is a subscriber (NFR-R7, MTR-004); otherwise zero work. When
  muted without a subscriber the chain is not computed at all; with a subscriber it is, so that the input
  meter stays alive, while the output is still digital silence.
- If nobody records the virtual microphone, the output port has no buffer and the chain does not run.
  While calibration is running or someone is subscribed to meters, the daemon asks the RT callback to run
  the input through a pre-allocated scratch buffer — meters and calibration work without a consuming
  application (needed for onboarding). The rest of the time an unused microphone still costs nothing.
- A slow client does not block the daemon: outgoing lines go through bounded queues, and events for it are
  dropped.
- IPC threads start after SIGINT/SIGTERM are blocked, so signals are handled only by the control loop and
  the daemon shuts down cleanly.

## Protocol (`macafon-ipc`, version 1)

JSON lines over the socket `$XDG_RUNTIME_DIR/macafon/macafond.sock` (directory 0700, socket 0600); on
Windows the named pipe `\\.\pipe\macafon-<user SID>` with a current-user ACL. The first request must be
`hello` with the protocol number; on a mismatch — `protocol_mismatch`.

| Method | Parameters | Reply |
|---|---|---|
| `hello` | `protocol`, `client` | `protocol`, `daemon_version`, `platform`, `capabilities` |
| `daemon.quit` | — | `null`; the daemon exits as on SIGTERM (BG-005) |
| `autostart.get` / `autostart.set` | — / `enabled` | `{enabled, method}`, `method` = `systemd`, `xdg_autostart` or `windows_run` |
| `mic.get` | — | `MicState` |
| `mic.set` | `settings` (the whole chain) | `MicState` |
| `mic.set_preset` | `preset` (`natural`, …, `user:<slug>`) | `MicState`; mute and bypass are kept |
| `mic.set_muted` / `mic.toggle_mute` | `muted` / — | `MicState` |
| `mic.set_bypass` | `bypass` | `MicState` |
| `mic.set_device` | `device` (`node.name` or `null`) | `MicState` |
| `mic.set_fallback_device` | `device` (`node.name` or `null`) | `MicState` |
| `mic.set_input_channel` | `channel` (`Auto`, `Left`, `Right`, `Mix`) | `MicState`; calibration pins the detected channel |
| `mic.set_default_source` / `mic.restore_default_source` | `confirmed: true` / — | `null`; the previous system input is kept in a journal and restored on exit |
| `devices.subscribe` / `devices.unsubscribe` | — | `null`, `devices.meter` events with `name` and `peak_dbfs`; separate measuring streams exist only while subscribed |
| `presets.list`, `devices.list` | — | lists; a preset has `id`, `name`, `builtin` |
| `presets.save` | `name` | `PresetInfo`; the current chain becomes a new preset, which becomes selected |
| `presets.update` | `id` | `PresetInfo`; overwrite a user preset with the current chain |
| `presets.rename` / `presets.duplicate` / `presets.delete` | `id` (+ `name`) / `id` / `id` | `PresetInfo` / `PresetInfo` / `null` |
| `presets.export` / `presets.import` | `id` / `document` | a `.macafon-preset.json` document / `PresetInfo` |
| `meters.subscribe` / `meters.unsubscribe` | `hz` (1–60) / — | `null`, then `meters` events |
| `calibration.start` / `calibration.cancel` / `calibration.get` | — | `CalibrationState` |
| `ab.record` | `max_seconds` (3–30, default 10) | `AbState` |
| `ab.stop_recording`, `ab.stop`, `ab.toggle`, `ab.get` | — | `AbState` |
| `ab.play` | `device` (output `node.name` or `null` — the default output) | `AbState` |
| `ab.select` / `ab.seek` / `ab.set_loudness_match` | `side` (`a`/`b`) / `position_ms` / `enabled` | `AbState` |
| `ab.waveform` | `points` (16–4096) | `{a, b}` — peaks 0…1 |
| `ab.outputs` | — | outputs with `kind` (`headphones`, `speakers`, `unknown`) and `speaker_warning` |
| `ab.export` | `side` (`a`/`b`), `path` | `null`; float32 WAV, 48 kHz, never overwrites an existing file |
| `mixer.get` | — | `MixerState`: channels, applications, Master, outputs ([mixer.md](mixer.md)) |
| `mixer.set_enabled` | `enabled` | `MixerState`; channels are created/removed, the system output moves to the default channel and back |
| `mixer.set_master` | `volume_pct?`, `muted?` | `MixerState` |
| `mixer.set_output` / `mixer.set_output_priority` | `device` (`null` — the previous system output) / `devices` | `MixerState` |
| `mixer.set_auto_classify` | `enabled` | `MixerState` |
| `channel.create` / `channel.update` / `channel.delete` | `name`, `color`, `icon?` / `id`, `patch` / `id` | `MixerState` |
| `channel.set` | `id`, `settings` (the whole chain and fader) | `MixerState` |
| `channel.adjust_volume` / `channel.toggle_mute` / `channel.set_default` / `channel.reorder` | `id`, `delta_pct` / `id` / `id` / `ids` | `MixerState` |
| `apps.move` / `apps.set_managed` / `apps.forget` | `app_key`, `channel` / `app_key`, `managed` / `app_key` | `MixerState` |
| `apps.icon` | `icon` (icon name, app id, binary or an exe path on Windows) | a `data:` URL or `null` (APP-003) |
| `hotkeys.get` / `hotkeys.set` | — / `action`, `shortcut` (`null` — unbind) | `HotkeysState` |
| `prefs.get` / `prefs.set` | — / `quantum_frames?` (64/128/256/512), `close_action?` (`tray`/`quit`) | `Preferences` (§9.5) |
| `doctor.run` / `doctor.fix` | — / `check` | Doctor checks (§9.6) / `null` |

Events: `meters` (levels in dBFS, gain reduction in dB, NS activity), `mic.changed` (any state change from
any client), `calibration.changed` (~10 Hz during the measurement, plus the result), `ab.changed` (~10 Hz
during recording and playback, plus every change), `presets.changed` (the new preset list),
`devices.changed` (the input list after a connect or disconnect, MIC-002), `prefs.changed`,
`hotkeys.changed`, `mixer.changed` (`MixerState` after any change of channels, applications or outputs)
and `mixer.meters` (channel and Master peaks together with `meters` while the mixer is on). The
`already_exists` error means the preset name is taken. Errors: `bad_request`, `protocol_mismatch`,
`invalid_params`, `not_found`, `busy`, `internal`.

TS types for the UI are generated from `macafon-ipc` (see [ui.md](ui.md)).

## Config (`macafon-config`)

`$XDG_CONFIG_HOME/macafon/config.toml` (`%APPDATA%\Macafon\config.toml` on Windows), `schema_version = 2`.
Version 1 is upgraded automatically; the original file is kept as `config.toml.bak`. The `[mic]` section:
`stereo` (default `false`), `device`, `fallback_device`, `preset`, `muted`, `bypass`, `output_gain_db` and
the module tables `[mic.compressor]` etc. Missing fields take their defaults. It is written atomically
(temp + fsync + rename) 500 ms after the last change and on exit (CFG-001). A corrupted, invalid or
newer-schema file is renamed to `config.toml.broken-<unix time>`, and the daemon starts with defaults.
`MicState.is_default_source` — Macafon Microphone is currently the system input per the
`default.audio.source` metadata (MIC-012). `MicState.config_recovery` holds the backup path and the reason
for a client notification; the UI shows a toast once per backup.

Profiles `[device_profiles."<stable ID>"]` contain the chain, preset, input channel and calibrated noise
threshold of each physical microphone. The ID is built from the serial or `device.bus-path` and the node
name; if hardware properties are missing, the node name is used. A profile is also loaded when switching
to the fallback microphone and back to the primary. `devices.list` returns `stable_id` and the device type
(`usb`, `builtin`, `headset`, `bluetooth`; no type means unknown).

The maintenance timer lives for the whole daemon run, including while waiting to reconnect to PipeWire.
Changes over IPC are saved with the same debounce even without an audio server; once it returns, a new
chain is built with the current settings.

`[audio] quantum_frames` (default 128) is the `node.latency` of the microphone and channel nodes and
changes live (`pw_filter_update_properties`); the graph takes the smallest request, so other applications
can make the quantum smaller. `[ui] quit_on_close = true` — a UI client disconnecting ends the daemon, like
`daemon.quit`.

`macafond run` flags (`--input`, `--fallback-input`, `--stereo`, `--preset`, `--engine`, …) override the
config for this run; later changes over IPC are saved together with them. `--config <path>` — a different
file.

Differences from the example in PRD §13: modules live in `[mic.<module>]`, not `[mic.modules.<module>]`;
percentages are 0–100, not fractions; application rules, the mixer and hotkeys arrive in M2.

## Logs (§13)

The daemon writes diagnostics to stderr and `$XDG_STATE_HOME/macafon/macafond.log` (if the variable is
unset — `~/.local/state/macafon/macafond.log`; `%LOCALAPPDATA%\Macafon\macafond.log` on Windows). The file
is appended to after a restart. Size-based rotation: the active file and four backups `.1`…`.4`, each up
to 2 MiB; `.1` is the newest. An event moves to the next file entirely if it does not fit; a single event
longer than 2 MiB is truncated. No ANSI codes in the file. New directories and files are created with
0700 and 0600 permissions respectively.

`RUST_LOG` sets the level for both outputs (default `info`). If the file is not available, the daemon
reports the error to stderr and keeps running. Writing is synchronous and only from ordinary threads: there
is no logging in RT callbacks (NFR-S1).

## Calibration (§6.5)

`calibration.start` (or `macafon ctl calibrate`) enables copying the raw microphone signal (before Gain)
from the RT callback into a 1 s `rtrb` queue; the control loop drains it every 100 ms. Steps are counted
in samples: 4 s of silence (CAL-001), then 6 s of speech (CAL-002). State:
`idle` → `measuring {step, progress, step_duration_ms}` → `done {report}` or `failed {reason, message}`.
After a successful measurement the settings are applied right away as an ordinary change (`mic.changed`),
and the measured noise floor is saved in `mic.noise_floor_dbfs` (reset when the microphone changes).
Errors: `clipped` (CAL-005, settings unchanged), `no_speech`, `disconnected` (the microphone
disappeared/changed, PipeWire restarted), `stalled` (2 s without audio). Rerun with the same method
(CAL-006).

In the config `mic.noise_floor_device` stores the device the floor was measured on: a calibration of the
fallback is not carried over to the primary after a restart. For old configs without this field the
result belongs to the saved primary microphone.

Analysis — `macafon_dsp::calibration::calibrate`, the CAL-003 rules and decisions not in the PRD:

| What | Decision |
|---|---|
| Noise floor | Average power of 20 ms windows, excluding the loudest 20% (a cough, a click) |
| Speech level | Average power of windows louder than the floor by 10 dB; less than 0.6 s of such windows — `no_speech` |
| Clip | ≥ 3 samples at or above −0.1 dBFS over the whole recording |
| Gain | −18 dBFS − speech level, 0.5 dB step, ±24 dB. Peaks (−6 dBFS from §6.3.1) are ignored — the limiter holds them |
| High-pass | 120 Hz if ≥ 50% of the noise power is below 100 Hz and the floor is above −75 dBFS; otherwise 80 Hz |
| Noise suppression | Linear: SNR 40 dB → 40%, 20 dB → 90%; with SNR < 20 dB — `suggest_studio` |
| Expander | Threshold = floor + gain + 6 dB, but not above speech + gain − 12 dB; ratio/range unchanged |
| Module enabling | Gain, HPF, NS, Expander are enabled; the rest of the chain, mute and bypass are untouched |
| Noise level (CAL-004) | By SNR: ≥ 40 dB low, 20–40 medium, < 20 high |
| SNR "after" | The same recording is run offline through the new chain; the floor after 1 s of settling |

## Recording and A/B (§6.4)

- **Recording (AB-001, AB-007).** The same raw signal capture as calibration (so they are mutually
  exclusive — `busy`). 3–30 s; it cannot be stopped before 3 s. The clip edges are smoothed over 10 ms so
  that the loop seam does not click. The clip is kept only in the daemon's memory. If the microphone
  disappears, a recording of at least 3 s is kept, otherwise discarded.
- **Playback (AB-002, AB-003, AB-008).** A separate `pw_stream` `macafon_ab` ("Macafon A/B test"), mono
  48 kHz, created only for the duration of playback. The target is `target.object` or the default output
  (connected by WirePlumber). B is a separate chain instance (`macafon_engine::ab_player`) with its own
  meters; every change of the live chain goes into it too. Mute does not apply to the test. The main
  virtual microphone works as usual.
- **A ⇄ B (AB-004).** Both sides are always computed; switching is a 20 ms linear crossfade. A is delayed by
  B's chain latency, so the position is shared and switching does not "jump".
- **Loudness match (AB-005), on by default.** A's gain = √(B power / A power), powers averaged over a 2 s
  window, ±24 dB, 50 ms smoothing. This is RMS, not LUFS. After an abrupt change of B's loudness the match
  converges in ~5 s — deliberately slow so that A does not "pump" after the compressor.
- **Waveform (AB-006).** A and B peaks on request; B is computed offline through the current chain with
  latency compensation (~0.15 s for a 10 s clip with NS).

## User presets (§6.3.10, §13)

- Files `<config directory>/presets/mic/<slug>.json` (next to `config.toml`, that is
  `$XDG_CONFIG_HOME/macafon/presets/mic`); the format matches the exported `.macafon-preset.json`:
  `{schema_version: 1, name, settings}`. Writes are atomic. Loaded at startup; broken files are skipped with
  a warning in the log and not deleted. Manual edits of the files are visible after a daemon restart.
- The ID is `user:<slug>`; the slug is taken from the name at creation and does not change on rename.
- Names are unique case-insensitively among user and built-in presets, 1–64 characters. Saving with a taken
  name is an `already_exists` error; duplicate and import add a " (2)", " (3)"… suffix.
- Built-in presets are read-only (`update`/`rename`/`delete` → `invalid_params`); they can be duplicated and
  exported.
- Mute and bypass are not part of a preset. `MicState.preset_modified` — the chain differs from the selected
  preset (UI: a "modified" badge, a Save button). Deleting the current preset resets `preset` to `null`.
- Import/export over IPC passes the document itself, not a path: the client reads and writes files (the UI
  through a dialog, `macafon ctl presets import|export`).

## Tray (§8.1)

A StatusNotifierItem over D-Bus through `ksni` (no GTK or tokio), in a separate thread; `--no-tray`
disables it. Menu items become ordinary requests to the control loop; the state is sent back to the tray on
every `mic.changed`. On Windows the tray uses `tray-icon` (see [windows-m3.md](windows-m3.md)).

- The icon is drawn in code (vector, anti-aliased, 16/22/32/48 px), no assets: the microphone in
  `--accent`; bypass — outlined; mute — crossed out in `--danger`; microphone not connected — a `--danger`
  dot (BG-002). States differ by shape, not only by color.
- Menu (BG-003): `Open Macafon`, `Mute microphone` ✓, `Bypass processing` ✓, `Preset ▸` (built-in and
  user), `Quit Macafon`. `Output device ▸` comes with the mixer (M2).
- Left click opens the UI, middle click — mute (BG-004). The UI is looked up as `macafon` next to the
  daemon, then in `PATH`.
- No tray host (the panel starts later, GNOME without the AppIndicator extension) — the daemon keeps running
  and shows the icon when a host appears. The BG-007 hint is a UI task.
- Tray tooltip: the preset (and "modified"), mute, bypass, a disconnected microphone.

## Autostart (§8.2)

- `packaging/systemd/macafond.service`: `WantedBy`/`PartOf=graphical-session.target` (the tray needs a
  desktop session), `After=pipewire.service wireplumber.service`, `Restart=on-failure`, `RestartSec=1`
  (NFR-S6). Quit from the tray ends the daemon with code 0 — systemd does not bring it back.
- `autostart.set` enables the unit (`systemctl --user enable`) if it is installed and the session reaches
  `graphical-session.target`; otherwise it writes `$XDG_CONFIG_HOME/autostart/macafond.desktop` with the
  path to the current binary. Only one method is active. `MACAFON_NO_SYSTEMD=1` forces XDG (tests, systems
  without systemd). On Windows — the `Macafon` value under `HKCU\…\Run` (AS-004).
- AS-003: in a private PipeWire `Macafon Microphone` appears 18–31 ms after start. Daemon RSS with the
  tray — 13 MB (NFR-R4).

## Not done

- A/B: the "output is speakers" warning (AB-002) needs an output list with device types — it comes with
  output selection; WAV export (AB-007, P2).
- Restoring the system default microphone on Quit (BG-005) — together with MIC-012.

## Checks

```sh
cargo test -p macafon-engine -p macafon-config -p macafon-ipc
cargo build --release -p macafond -p macafon-cli
python3 tools/tests/control-smoke.py
python3 tools/tests/tray-smoke.py --icons .artifacts/screenshots/tray
```

`tray-smoke.py` reads the SNI properties through `gdbus` in a private D-Bus session, clicks menu items
(mute, bypass, preset, middle click, Quit) and saves the state icons as PNG.

`control-smoke.py` in a private PipeWire checks: refusal of a second daemon, live mute/unmute and gain, a
preset change with a crossfade and `ProcessLatency` updated in the graph, meters without a consuming
application, protocol errors, calibration on a synthetic recording (noise −60 dBFS, "speech" −24 dBFS)
without a consumer, recording and A/B (levels of A, B, B after a live chain change, loudness matching,
waveform, stream removal), user presets (save, the modified flag, a duplicate name, export → import,
duplicate, rename, delete), saving the config and loading it after a restart, removing the socket on exit.
The fallback microphone is checked with real audio from two synthetic inputs of different levels: the
primary's priority, switching to the fallback and back, silence when both are disconnected, interrupting
calibration/recording on an input change, explicitly disconnecting the input and keeping the fallback
after a restart.
Stereo is checked on recorded samples: processed FL/FR are equal, a single connected channel works, mute,
meters without a consumer, physical input disconnect/return, the format saved in the config, recovery after
a PipeWire restart and the return to mono with the flag. While PipeWire is down, mute is changed: the
config is saved without the daemon exiting in under 2 s, and there is digital silence after the server
returns. A `macafon-backend-pipewire` test under `assert_no_alloc` checks duplication without repeated DSP,
partial connections, silence and scratch processing without consumers.

## Studio budget control

The daemon subscribes to the PipeWire Profiler only while Studio runs. The `finish − awake` duration of our
own node is compared with the `quantum × clock.rate` budget. Three consecutive completed cycles above 80%
trigger the switch to Lite, saving the device profile and a `mic.noise_fallback` event. Measurements are
taken on the control loop; no clocks, system calls or IPC were added to the audio callback.

Clock positions and the completion timestamp prevent reading the same cycle twice. Skipped measurements, a
driver change and a new subscription reset the streak. The subscription is removed when the stream stops /
Studio is turned off, and recreated on reconnect. The DFN3 worker keeps checking its own budget and missed
results independently. If the Profiler is disabled on the server or the client module is missing, this
worker detector stays active, but the full callback time is not available.

On Windows the capture thread times the chain per WASAPI packet instead (see
[windows-m3.md](windows-m3.md)).
