# Windows port (M3)

State as of 2026-10-16: the daemon, the UI (Tauri) and the per-user installer work on Windows; the MVP
path "install → VB-CABLE → audio in CABLE Output" is in place (see the later sections). Below is the
history of stages M3.0…M3.6. Test machine: Windows 11 25H2, HyperX SoloCast (USB), VB-CABLE 3.3.1.7.

## M3.0: spike

```sh
cargo run --release -p macafon-backend-windows --example spike -- devices    # endpoints, hardware IDs, periods
cargo run --release -p macafon-backend-windows --example spike -- defaults   # default devices per role
cargo run --release -p macafon-backend-windows --example spike -- sessions   # application audio sessions
cargo run --release -p macafon-backend-windows --example spike -- run --input SoloCast --seconds 60
```

`run` writes into `CABLE Input` by default; `--output '<part of a name>'` selects another device,
`--low-latency` — the minimum render period, `--probe-every-s 10` — a measurement of the added latency.
`probe <input> <input>` compares two capture devices (a check of the measurement itself: an input against
itself gives 0.00 ms). `set-default` changes the default device and restores the previous one after a few
seconds.

### What was done

- VB-CABLE detection by the hardware ID of the `VBAudioVACWDM` adapter (service `VBAudioVACMME`), without
  endpoint names (WIN-MIC-001). The adapter is the parent of the `SWD\MMDEVAPI\<id>` devnode. VB-CABLE 3.3
  provides two render endpoints on one cable: `CABLE Input` (form factor Speakers) and `CABLE In 16ch`
  (LineLevel); Speakers is chosen. Other VB-Audio devices are flagged but not used.
- Default devices: read through `IMMDeviceEnumerator`, written through `IPolicyConfig` (the Windows 7+
  vtable) — the basis of MIC-012/WIN-CH-007. Journal and restore — M3.1.
- Audio sessions on all render endpoints: process, exe, volume, mute, peak (the basis of
  WIN-CH-001/002/004), read-only.
- The microphone path: WASAPI shared event-driven capture (MMCSS "Pro Audio") → `MicRt` → drift
  compensation buffer → WASAPI render, mono duplicated into stereo. Format 48 kHz float32; Windows does
  resampling and channel mixing of the device (`AUTOCONVERTPCM`). RT sections under `assert_no_alloc`.
- `macafon-engine::drift`: an SPSC buffer + a cubic resampler with a PI controller (≈ 20 s, up to ±0.5%).
  The controlled quantity is the total "buffer + render queue" latency: render sometimes takes two periods
  at once, and controlling the buffer alone caused bursts of underruns. The level target is the render
  buffer + one capture period + 2 ms: capture packets arrive once per capture period, however short the
  render period is.
- Render at the engine's minimum period (`IAudioClient3::InitializeSharedAudioStream`) if the device mix
  format is 48 kHz float; otherwise the regular path.
- Added latency measurement (`probe`, the basis of WIN-MIC-008): simultaneous recording of the physical
  microphone and `CABLE Output`, aligned to the QPC timestamps of WASAPI packets, cross-correlation. Both
  streams start from the same device samples, so even the microphone's own noise correlates; the result is
  how much an application loses by recording Macafon instead of the microphone (the NFR-L1/L6 definition).
- IPC: `macafon-ipc::transport` — on Windows the named pipe `\\.\pipe\macafon-<SID>` with a protected
  "current user only" DACL through `interprocess` (overlapped I/O: events are written from another thread
  while a read is blocked). A second owner of the name is rejected. `Client` and `macafon ctl` moved to
  `Endpoint`; Linux keeps the Unix socket.

### Measurements

| | Result |
|---|---|
| Default WASAPI engine period | 10 ms on all devices; minimum (IAudioClient3): `CABLE Input` 2.67 ms, HyperX 10 ms, `CABLE Output` 10 ms |
| Safe buffer level | render buffer (22 ms) + period (10 ms); less — underruns in some runs |
| Added latency, render period 10 ms | **124–138 ms** measured (correlation ≥ 0.96); our own part ≈ 44 ms by components |
| Added latency, `--low-latency` (2.67 ms) | **82–88 ms** measured (correlation ≥ 0.87); our own part ≈ 28 ms |
| The rest — the VB-CABLE buffer | ≈ 58–90 ms with the default driver settings; depends on the size of writes into the cable |
| HyperX ↔ VB-CABLE drift | tens of ppm after settling; more precise after a soak test |
| Underruns / dropouts | 0 in 40–60 s in both modes |

**NFR-L6 (≤ 20 ms) is not met.** Most of the latency is the VB-CABLE internal buffer (the Max Latency
setting in `VBCABLE_ControlPanel`, not stored in the registry). The measurement has to be repeated with a
smaller cable buffer; our own part is limited by the USB microphone's capture period (10 ms) and the render
buffer. If the target is still unreachable, revisit NFR-L6 in ADR-0004.

Not done in M3.0: measurements with different `VBCABLE_ControlPanel` buffers, a one-hour soak,
`IPolicyConfig` on Windows 10. The numbers are not in ADR-0004 yet.

## M3.1: platform layer

- Hot-plug and live device changes (WIN-MIC-006): the capture and render threads survive a device
  disappearing or changing its format and reopen it every 500 ms; input and output switch without a
  restart (`MicrophonePipeline::set_input/set_output`). Without a microphone the cable gets silence;
  without the cable the chain and meters keep running (WIN-MIC-001). Checked by switching HyperX → Steam
  Streaming Microphone and to a non-existent ID; physically unplugging the USB microphone has not been
  checked yet.
- Default device journal (MIC-012, WIN-MIC-005): the original defaults per role are written to disk before
  a change and never overwritten; restore only where the default is still ours; if the original device is
  disconnected, the journal stays until the next start.
- Paths: config `%APPDATA%\Macafon`, state and journals `%LOCALAPPDATA%\Macafon`
  (`macafon_config::state_dir`).
- The shared backend contract is the `macafon-backend` crate (`MicrophoneHost`, `MixerHost`, `Node`,
  `PlaybackStream`, `ControlSender`, a graph snapshot for Doctor). The PipeWire and Windows crates implement
  it; the daemon picks the platform in `apps/macafond/src/platform/{linux,windows}.rs`.
- `macafond` and `macafon` build and run on Windows. The control loop is
  `macafon-backend-windows::daemon`: polling the host every 100 ms, rescanning endpoints every second (the
  cable is hidden, WIN-MIC-009), A/B through its own WASAPI thread; Ctrl+C / console close / logoff /
  shutdown stop it after restoring the defaults (WIN-MIC-005). The control server is a named pipe. Log —
  `%LOCALAPPDATA%\Macafon\macafond.log`.
- Channels as volume groups (WIN-CH-001…004, 007, 008): applications from `SessionWatcher` are passed to the
  host as streams (by exe), the daemon's existing `Mixer` provides the channel and faders
  (`MixerHost::app_mix`), session volume = curve (channel × app × Master). A change in the Windows volume
  mixer becomes the app fader; app peaks feed the channel meters; the chosen output becomes the default
  render device while channels are on; disabling the mixer and Quit give back volumes and the default.

### Checked on the test machine (Windows 11, HyperX SoloCast, VB-CABLE)

| | Result |
|---|---|
| Audio in `CABLE Output` (`spike probe`, raw preset) | latency 85.5 ms, correlation 0.997; mute — silence (correlation 0.00) |
| `ctl status / devices / doctor / meters` over the named pipe | work; the cable is not in the microphone list |
| Live microphone change (`ctl device`) | HyperX → Steam Streaming Microphone → HyperX without a restart |
| `ctl set-default --confirm` / `restore-default` | `CABLE Output` for all roles and back |
| Quit after set-default | the default microphone restored, the journal removed |
| Killing the process after set-default | the journal stayed; the next start restored the default |
| A/B (`ab record` 3 s, `ab play --device`) | plays, the position advances, `ab stop` stops |
| Mixer: `ctl mixer on`, `channel main volume 50` | grouping by exe, Discord/Telegram → voice; sessions at 12% (0.5³); channel mute |
| `ctl mixer off` | session volumes given back (the journal stays only for applications that have exited) |

### Application icons

The full exe path goes from the session through `PlaybackStream.icon_name` into `AppState.icon` (APP-003,
WIN-CH-001). `apps.icon` extracts a 32×32 icon with `ExtractIconExW`, draws it into a BGRA DIB and returns
a PNG data URL; the cache and the miss TTL are kept. It uses the light `png` crate (MIT/Apache-2.0) that was
already in the lockfile, without `image`. AppX manifests are not read: for WindowsApps the exe icon is used,
and without one — the existing UI fallback. System sounds keep their pseudo-icon.

### Not done / limitations

- Channel meters update every 100 ms (polling session peaks).
- Console close, logoff and shutdown were not checked manually.
- No one-hour soak.

## M3.4: mixer backend

The platform part is in `macafon-backend-windows::mixer`. `SessionWatcher::start(state_dir)` owns a separate
MTA thread; `apps()` returns groups by exe name (case-insensitive), system sounds — `<system-sounds>`. The
`IAudioSessionNotification`, `IAudioSessionEvents` and `IMMNotificationClient` subscriptions track new
sessions, states and devices on all active render endpoints (WIN-CH-001, APP-002/011). The creation callback
holds a COM reference and passes it through an atomic queue; other callbacks update atomics, with no waiting
and no disk I/O. Events are available through `try_event()`; COM interfaces are not handed to the caller.

`set_app(exe, MixSettings)` takes channel/app/Master in percent and three mute flags. The curve comes from
`macafon_dsp::channel::volume_gain`; the setting applies to all sessions of the exe, including future ones
(WIN-CH-002, APP-005); a session whose first write fails is retried on the next scan. An external event
context produces `ExternalVolume` with the app fader recomputed backwards, without writing back into the
session (WIN-CH-003). With a zero channel/Master the app fader cannot be observed: the previous value is
kept. Meters are polled only on request: the maximum of the app's session peaks and separate
`output_peaks()` of all devices (WIN-CH-004); choosing channels/output stays with the caller (WIN-CH-009).

`session-volumes.json` is kept in the given state directory; a write with `sync_all` and an atomic file
replace precedes every Windows change, and the new value is journaled as `pending` until it is confirmed.
`restore()` clears the rules and gives back the original volume/mute independently of each other, only
while the respective value is still ours (the confirmed or the pending one). Restore also runs at start and
on Drop (WIN-CH-008). With `managed = false` the app's original volume/mute are restored on the next
control tick (up to 100 ms): `restore_app()` removes the rule for future sessions and gives back only this
exe's records. Other applications are not affected, external changes are kept. Re-enabling management
applies the faders again (WIN-CH-008). The restore key is exe + endpoint; the instance ID is a hint for a
live session. If several sessions of one exe on one endpoint had different original values and their
instance IDs changed, an unambiguous restore is impossible: the records are kept. Missing sessions also stay
in the journal until the next start/`restore()`; the `RecoveryPending` event reports their number. One state
directory must have one owner.

Manual command:

```sh
cargo run -p macafon-backend-windows --example spike -- mixer --seconds 15
cargo run -p macafon-backend-windows --example spike -- mixer --exe steam.exe --volume 50 --mute --seconds 5
```

`--volume` sets the channel (app/Master = 100%), `--state-dir` selects the journal directory; by default
`.artifacts/mixer-state`. The command prints groups, events and peaks, and calls `restore()` explicitly on a
normal exit; restore errors are visible before Drop.

Checked natively on Windows 11: fmt, clippy, tests including composition, inversion and the journal with a
fake session store. `spike mixer` found 10 Steam sessions and 12 system sound sessions; mute applied to the
whole Steam group, snapshots before/after restore matched. A temporary local harness with a quiet WAV
checked a new active session (peak ≈ 0.00916), the device meter, scalar 0.125 at channel 50%, mute and an
external change of the scalar to 0.064 → app 80%. The external value was not overwritten on restore. A
setting made before the session was created applied automatically. After the harness exited without Drop,
the next process restored `(0.125, mute=true)` to `(1.0, mute=false)` from the journal.

Not checked: physical device hot-plug, Windows 10 and the full application matrix from §14.4. Effects and
audio redirection were not added (WIN-CH-006/009/010).

## M3: outputs and Doctor facts

`OutputPolicy::scan` takes ordered endpoint IDs and the active devices: it picks the first available render
device, excluding all endpoints of the base VB-CABLE (WIN-CH-007, OUT-001/002/004, WIN-MIC-009). An explicit
choice is a list of one ID. All three roles change through `journal::set_default`; the journal is restored
on Quit/next start only where the default is still ours. Endpoints chosen by applications themselves are
not changed. An external change of any role while the previous priority endpoint is available returns
`ManualChange` and pauses the automation until the list's winner changes or an explicit `reset`. A change
of the device only, without an ID change between scans, cannot be distinguished. Switching to "Follow system
default" (OUT-003) calls `follow_system_default` (conditionally restores the original roles) and then passes
an empty list. One journal must have one owner; the journal is restored at start before the policy is created.

Doctor reports the installed and active endpoints of the base cable, separately ignored VB-Audio devices,
the mix/device format and whether resampling is needed, WASAPI periods, active foreign sessions and default
render roles on the cable (WIN-MIC-004/007/008). CABLE In 16ch is counted too; idle sessions are excluded,
muted/zero peak do not exclude the owner of an active stream. `fix_cable_default` picks the available
original physical render device from the journal, otherwise the first available render device on a
recognized physical bus (USB/HDAUDIO/PCI/Bluetooth/ACPI), and writes through the journal. Unknown and
ROOT/SWD software adapters are never picked automatically. Individual format/period read errors are kept in
the facts.

The Max Latency recommendation: lower the buffer in VBCABLE_ControlPanel, repeat the probe and check for
dropouts. With the default settings the cable contributed ≈58–90 ms; the full measurement at the minimum
render period was 82–88 ms (see the table above). There is no verified numeric Max Latency value yet; the
Doctor constant says so explicitly. Engine periods are not presented as the actual full latency. Panel
settings are neither read nor changed.

```sh
cargo run -p macafon-backend-windows --example spike -- outputs
cargo run -p macafon-backend-windows --example spike -- outputs --priority '<endpoint-id>' --apply --restore-after-s 3
cargo run -p macafon-backend-windows --example spike -- doctor --input SoloCast
```

`outputs` without `--apply` only reads. When applying, the journal is kept in `.artifacts/output-state` by
default; an unfinished previous run is restored before a new one, and after the wait a conditional restore
runs, including after a partial write error.

A real `spike doctor --input SoloCast`: the cable is installed, 3/3 registered/active endpoints found, other
VB-Audio — 0. All three checked devices are 48000 Hz, 2 channels; mix — 32 bit, device — 24 bit for CABLE
Input/Output and 16 bit for the HyperX SoloCast. No resampling needed. Default/min/max periods: CABLE Input
10/2.67/10 ms, CABLE Output and SoloCast 10/10/10 ms. Initially 0 foreign writers, 0 default roles on the
cable. An external PowerShell/WinMM harness wrote a quiet tone straight into CABLE Input without changing
the default: Doctor detected `powershell.exe`, active=true, muted=false, peak≈0.0010. After the harness
ended, writers were back to 0. `outputs --apply --restore-after-s 1` moved the roles to Realtek Speakers and
restored the original Console/Multimedia = Sonar Gaming, Communications = Sonar Chat; the final ID snapshot
matched, the journal was removed.

## M3.5: hotkeys and autostart

Windows hotkeys (HK-001…003, §12.2.3) are registered by `global-hotkey` on a separate thread with a Win32
`GetMessageW` loop. The `Accel` translation shared with X11 is kept; rebinding goes through the existing
channel. Presses and releases reach the control loop, including PTT; rebinding a held key sends a release.
Registration conflicts show up as an error of the respective action. IPC got `HotkeyBackend::Windows`.

Autostart (AS-004) reads/changes only the `Macafon` value in
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`: `"<macafond.exe>" run`, without UAC. It is never
enabled on its own; the IPC method is `windows_run`.

Manual check on Windows 11: `ctl hotkeys set mic_mute_toggle Ctrl+Alt+F9`, PTT on Ctrl+Alt+F10/F11,
synthesized through `SendInput`, reading `ctl status`. Mute toggles once per press; PTT releases, including
rebinding while held. A taken Ctrl+Alt+F12 returns a `bypass_toggle` error. `ctl autostart on/off`
creates/removes the exact registry command; a repeated `off` succeeds. Linux `control-smoke` and
`hotkeys-smoke` kept their behavior (XDG autostart, portal, toggle/PTT, presets, volume, OSD). PTT belongs
to HK-001; HK-005 in the PRD is a sound cue, which was not added here.

## M3.5: Windows tray and background shutdown

The daemon uses `Shell_NotifyIconW` from the existing `windows` crate, without a new crate and without
winit/tao. Its own thread owns a hidden window, the message loop, the HICON and the menu. The shared
procedural ARGB renderer is kept for Linux; Windows builds RT_ICON from it. The normal/mute/bypass/
disconnected states and the 300 ms transition follow BG-002. The menu mirrors Linux (BG-003), including
Mixer and radio items for presets/outputs; a left click starts `macafon-ui.exe` (the Tauri `[[bin]]` name
and the package's `mainBinaryName`) next to the daemon, then from PATH; `macafon.exe` is the CLI and the
tray does not start it. A middle click toggles mute (BG-004). After an Explorer restart the icon is
registered again (`TaskbarCreated`). State updates and notifications go through a queue; the control loop
does not call the Shell.

BG-001 and HK-006 on Windows use a balloon of the same icon (`NIF_INFO`), without an AppUserModelID/shortcut
registration. A new OSD removes the previous one first; several OSDs waiting in the queue are merged into the
last one. The display time and actual visibility are decided by Windows (including notification settings /
Do not disturb). With `--no-tray` or an unavailable tray the message stays in the log.

`macafond.exe` is built with the GUI subsystem in all Windows profiles: HKCU Run and launching from the UI
do not create a console. At the start of `main`, before clap and logging, `AttachConsole(ATTACH_PARENT_PROCESS)`
is called: launching from a terminal keeps the device and diagnostic output, and a new console is never
created. Some shells do not wait for a GUI exe to finish; sequential scripts should use `Start-Process -Wait`
or wait for the process explicitly.

The backend creates a separate hidden top-level window even with `--no-tray`. `WM_QUERYENDSESSION` allows
the shutdown but does not stop audio: a logoff can still be cancelled. `WM_ENDSESSION(TRUE)` sets the same
stop flag as the console handler and waits up to 4 s for the control loop to stop WASAPI, restore session
volumes and the default output, and run cleanup with restoring the default microphone and saving the config
(WIN-MIC-005, WIN-CH-008, BG-005). FINISHED is set after cleanup. If the limit is exceeded, the journals stay
for a restore at the next start. The console handler is kept for Ctrl+C / closing the terminal. A real
logoff/shutdown is not triggered automatically so as not to end the user session.

Checks: fmt, clippy and tests of `macafond` and `macafon-backend-windows`, a release build; `cargo check -p
macafond` in Ubuntu/WSL. A targeted test sends the hidden window QUERY, a cancelled ENDSESSION and a
confirmed ENDSESSION; the last one waits for a simulated cleanup. The release daemon ran with a separate
config and LOCALAPPDATA inside `.artifacts`: the Shell accepted the icon and the first-close balloon, IPC
confirmed SoloCast and mute/bypass changes, Quit ended the process, all six default roles matched
before/after. Visual checks of the tray/menu, launching the UI by a click, the OSD from a physical hotkey and
a real logoff/shutdown are not confirmed.

## M3.2, M3.3, M3.6: UI, Doctor and installer

**UI bridge.** `apps/macafon-ui/src-tauri/src/bridge.rs` works through `macafon_ipc::transport` (a Unix
socket or the named pipe `\\.\pipe\macafon-<SID>`) and starts `macafond.exe run` without a console
(`CREATE_NO_WINDOW`). The release UI build uses the GUI subsystem. The bridge test passes on Windows with a
fake daemon on a named pipe.

**Platform capabilities (M3.2).** `hello` returns `platform` and `capabilities` (`virtual_mic_device`,
`stereo_option`, `channel_fx`, `graph_quantum`); the UI hides what is unavailable by flags.
`MicState.virtual_mic_ready` — the cable was found (WIN-MIC-001); the daemon learns it when rescanning
endpoints. Mock: `?platform=windows`, `&cable=missing`.

**Microphone and onboarding (M3.3).** An "Install VB-CABLE" step after Welcome (WIN-MIC-002): instructions,
`Open download page` opens only `https://vb-audio.com/Cable/` (a Tauri command with a fixed URL), a
donationware note. While there is no cable, the step is remembered in `localStorage` and onboarding
continues from it after a reboot; once the cable appears, the step shows it is ready. "Use it" and Getting
Started name `CABLE Output (VB-Audio Virtual Cable)` (WIN-MIC-003; no picture). The Microphone screen without
a cable shows a banner with an install button; the default microphone toggle is unavailable in that state.
Quit warns that CABLE Output will go silent (WIN-MIC-005). Mixer: a Channel button instead of EQ/FX, and
"Channel effects are available on Linux" is shown once (WIN-CH-006); the disabled mixer text describes
volume groups.

**Doctor.** New checks `vb_cable`, `cable_format`, `cable_writers`, `cable_default_output`, `cable_latency`
(WIN-MIC-001/004/007/008) from `diagnostics::collect`; they come first. `doctor.fix` for
`cable_default_output` moves all render roles to a real device (preferably the original one from the mixer
journal) — this is the user's fix, Quit does not roll it back. `cable_latency` shows the minimum engine
periods and advice to lower Max Latency; it is not the full latency.

**Installer (M3.6).** `python tools/package-windows.py` → NSIS
`target/release/bundle/nsis/Macafon_<version>_x64-setup.exe` (~16 MiB): `installMode = currentUser`, no UAC,
into `%LOCALAPPDATA%\Macafon`; it contains `macafon-ui.exe`, `macafond.exe`, `macafon.exe` and the licenses.
VB-CABLE is not included. Hooks (`packaging/windows/hooks.nsh`): before an update `macafon ctl quit`, then
wait (up to 15 s) until `macafond.exe` is no longer locked, and start `macafond run` again after the copy if it
was running; `--install` runs the installer in passive mode (`/P`, plus `/R` on a first install to open the UI, `/UPDATE`
afterwards: without it a passive reinstall of the same version runs the old uninstaller and its cleanup);
before
uninstall `macafond cleanup` — Quit the daemon, restore the default microphone/output and session volumes
from the journals, remove the Run value. If cleanup fails, the uninstaller asks "Uninstall anyway?"; a
silent uninstall stops. **Deviation from PRD §12.2.3:** NSIS instead of MSI (WiX), because the Tauri MSI
installs per machine and needs administrator rights, and "no administrator rights" matters more than the
format. SignPath signing is not configured.

Checked on the test machine: a silent `/S` install without UAC; the installed `macafon-ui.exe` started the
daemon itself and connected over the named pipe, onboarding showed the VB-CABLE step; `ctl doctor` printed the
cable checks (all ok, periods 10.0/2.7 ms); a silent uninstall stopped the daemon (`Quit requested` in the
log), removed the binaries and the Run value. The log and state journals stay in the directory: the install
directory is the same as `state_dir`.

Not checked: `doctor.fix` on a real system (the default output was not moved to the cable so as not to touch
the test machine's audio; the logic is covered by `fix_cable_default` fake tests), tray menu clicks and a
real logoff, building `macafon-ui` on Linux (no webkit/dbus-dev in WSL), Windows 10, a clean system without
VB-CABLE, a one-hour soak, the application matrix.

## NFR-S3: Studio overload on Windows

While NS Studio is on, the capture thread times the chain for every WASAPI packet (`Instant`/QPC, no
waiting) and puts a "time / packet duration" pair into an `rtrb` queue. Every 100 ms the control loop hands
the measurements to `MicrophoneHost::processing_budget` — the same path as the PipeWire profiler: > 80% of
the budget for 3 packets in a row → fallback to Lite and an event in the UI. Enabling the measurement and a
queue overflow produce a gap sample that resets the streak. Without Studio the measurement is off. Checked by
a targeted test on the SoloCast (`cargo test -p macafon-backend-windows chain_timings -- --ignored`); a real
overload was not reproduced on the test machine.

## Windows port audit (2026-10-16)

Fixed after a bug hunt (WIN-MIC-005, WIN-CH-007/008):

- The default device and session volume journals write the new value as `pending` before the COM call; on an
  error or a crash both the previous Macafon value and the new one are restored. The journal is flushed to
  disk (`sync_all`) before the settings change.
- A `GetDefaultAudioEndpoint` error is no longer taken as "no default" and no longer erases the journal.
- `macafond cleanup` waits for the daemon to exit, holds the pipe name until the restore is finished and
  returns an error if the microphone, output or volumes could not be given back. The uninstaller then asks
  "Uninstall anyway?"; a silent uninstall stops (`python tools/tests/windows-uninstall-hook.py`).
- A new session whose settings were not applied gets them on the next scan.
- Switching to "Follow system default" immediately gives back the default output set by Macafon.
- No event handles leak on a WASAPI initialization error; the render thread stops if the capture thread did
  not start.

Checked: clippy/tests of `macafon-backend-windows`, `macafond`, `macafon-apps`; the release daemon with
separate APPDATA/LOCALAPPDATA — applications with icon paths, no warnings in the log, a normal Quit.
Re-applying to a new session is not covered by a test (needs real COM).

## Mixer: "the audio thread is not consuming changes" (2026-10-16)

Dragging a channel fader several times on Windows showed a stream of "the audio thread is not consuming
changes; try again" errors. Windows has no channel nodes, so the daemon drops the `ChannelRt` it builds for
each channel, but `ChannelControl` kept queueing every fader change: after 64 changes the queue was full and
each further change returned `Busy`. `ChannelControl::apply` now only stores the settings when the audio side
is gone (`rtrb` abandoned). Regression test: `cargo test -p macafon-engine channel`.

## Tray: clicks on the icon did nothing (2026-10-17)

Right-click opened no menu (left and middle clicks were lost the same way). Explorer delivers the
`Shell_NotifyIconW` callback as a sent message, so it goes straight to the window procedure and never reaches
the `GetMessageW` loop that handled it; the procedure passed it to `DefWindowProcW`. The procedure now re-posts
the callback to the loop, which opens the menu outside Explorer's send. Checked by clicking the real tray icon
through UI Automation: the popup menu (`#32768`) opens and belongs to the daemon. Not covered by a test
(needs Explorer).

## Hotkey popup instead of notifications (2026-10-17, HK-006)

Volume hotkeys raised a tray balloon per step, which piled up while a key was held. Hotkey actions now show
no system notifications on Windows; `osd/popup.rs` draws Macafon's own popup instead:

- A layered, click-through, topmost, non-activating window on a `macafon-osd` thread, placed at the right
  edge of the work area of the monitor with the foreground window (else the cursor), per-monitor DPI.
- Direct2D renders into a WIC bitmap (icons through Direct2D SVG, Lucide like the UI; text through
  DirectWrite, Segoe UI Variable), `UpdateLayeredWindow` presents it; frames pace on `DwmFlush` only while
  something moves, otherwise the thread sleeps.
- Springs: the entrance slides and scales in with a light overshoot, the level follows each step, holding
  a key at 0 or 100 % stretches the slider; the popup leaves 1.6 s after the last change. With Windows
  "Animation effects" off everything snaps.
- Volume: the channel's color, icon and name; mute crossfades the percentage to "Muted". Mute, bypass and
  preset: a card with an icon, what changed and its new state.

- Settings → Hotkey popup (`[ui.osd]`, `prefs.set { osd }`): style `vertical` (Android-like, `left`/`right`)
  or `horizontal` (Windows 11-like bar, `top`/`bottom`), and `offset_pct` (0…40 % of the screen) toward the
  center. A mismatched edge falls back to the style's default. Every change shows a sample popup; a visible
  popup moves at once. The window never leaves the work area: with an offset it grows toward the edge so
  the slide-in is not clipped. The UI shows the section only with `Capabilities::hotkey_popup`.

Previews: `cargo test -p macafond osd_previews -- --ignored` writes PNGs to `.artifacts/screenshots/`.

## Tray: the menu froze on screen (2026-10-17)

The fix above re-posted the callback under the same message id. While the menu's modal loop ran it
dispatched that posted copy back to the window procedure, which posted it again: the queue never emptied,
and posted messages come before input and painting, so the open menu stayed on screen and ignored clicks.
The procedure now re-posts under a separate id (`QUEUED`) that only the tray loop handles; inside the menu
loop such copies are dropped.

## Quick panel, channel switch, default hotkeys (2026-10-17, CH-010, HK-007, HK-008)

- `macafon-ui --quick-panel` opens only the quick panel window (`quick_panel.rs`); a running UI receives
  the argument through the single-instance plugin and toggles it. The main window has `create: false` in
  `tauri.conf.json` and is built on demand. The panel sits at the bottom right of the work area of the
  monitor under the cursor, sizes itself to its content and closes on focus loss. The daemon calls
  `AllowSetForegroundWindow(ASFW_ANY)` before launching the UI, so the panel gets focus over a game.
  The panel's store does not subscribe meters: it shares the bridge connection with the main window.
- A channel switched off (`enabled = false` in `[[mixer.channels]]`) has no node; `MixerConfig::effective_channel`
  sends its apps to the default channel.
- `Config::offer_default_hotkeys` runs once per `HOTKEY_DEFAULTS_REVISION` at daemon start.
  Revision 2 (owner request, 2026-10-18) applies only to an unoffered empty hotkey map or the
  exact full revision-1 map. Any custom, partial, secondary or cleared map stays unchanged;
  partial maps skipped for an old layout are conservatively treated as user configuration.
  Missing channel IDs receive no new defaults. Main uses `RCtrl+PageUp/PageDown`, Voice
  `RAlt+PageUp/PageDown`, and the quick panel `LCtrl+LAlt+Z`; microphone and Music defaults stay.
  Side-specific shortcuts use the existing `WH_KEYBOARD_LL` hook. An RAlt binding without
  Ctrl ignores Windows' AltGr synthetic LCtrl; LCtrl+LAlt does not match. The recorder uses the
  same rule (physical LCtrl+RAlt is indistinguishable from AltGr). Generic Ctrl/Alt retain
  either-side behavior; conflict checks include AltGr overlap. Linux uses modifier families
  for both registration and conflict checks, so side tokens fall back to either side.
  `types_with_altgr` checks every installed layout with `ToUnicodeEx` (Ctrl+Alt held), except
  explicit LAlt bindings, which cannot be triggered by AltGr.


## Quick panel: black main window and two modes (2026-10-17, HK-007)

Open Macafon created the main WebView2 window inside a synchronous Tauri IPC command. On Windows,
WebView2 initialization waits for the event thread that the command itself blocks, leaving a black,
unresponsive window (also warned about in Tauri's WebviewWindowBuilder documentation). Window creation
now runs on a worker for panel commands, single-instance requests and initial launches; the panel closes
only after the main window is built. Focus-loss closing is guarded during that creation because
WebView2 can take focus before Tauri registers the new window. Windows tray right-click opens the compact
`--quick-panel` mode with Close and Quit Macafon; the hotkey and CLI use `--overlay` on the foreground monitor. Switching
modes replaces the shared `quick-panel` window without exiting the UI between windows.

Verified with the installed NSIS build: Open Macafon renders the main UI, normal launch and the Explorer
icon default action work, both modes toggle and replace one another, Close/Esc work, and Quit stops the
daemon and UI. Right/middle tray callbacks were injected through Win32; physical right-click remains
unverified because cursor injection was unavailable in this automation session.

## Panels open at once: a warm UI process (2026-10-17, NFR-R5 deviation)

Each tray right-click or overlay hotkey started `macafon-ui` from scratch (Tauri + WebView2, ~370 ms to a
window) with the "app starting" cursor next to the pointer. Now:

- The daemon starts `macafon-ui --background` after its pipe is ready (`ui::prewarm`) with
  `CreateProcessW(STARTF_FORCEOFFFEEDBACK)`, so no busy cursor. The UI creates the panel window hidden and
  keeps it; closing the main window keeps the process (`ExitRequested` is prevented). It exits when the
  daemon goes away (status Connected → not connected without a main window), so `ctl quit`, Quit and
  upgrades still end it; the installer also waits for `macafon-ui.exe` to unlock.
- The daemon asks a connected UI with the event `ui.show { target: main | tray_panel | overlay }` instead
  of launching a process; without a UI it falls back to the launch arguments.
- One panel window for both layouts (`Panel.svelte`): `panel://show` switches the layout, the page
  measures it and `quick_panel_fit` shows the window. A click on the tray icon that took the focus from
  the panel closes it instead of reopening (400 ms grace).
- Closing the main window sends `meters.unsubscribe`, `devices.unsubscribe` and `ui.window_closed` (the
  daemon's BG-001 hint / quit-on-close moved from "UI disconnected" to this request).

Measured on the owner's machine: hotkey 13–14 ms, tray right-click 13–38 ms, close 11–29 ms; ~190 MB private
for the warm UI. WebView2's `MemoryUsageTargetLevel = Low` while hidden saved nothing measurable and cost
~100 ms per open, so it is not used.
