# M1: UI (Tauri v2 + Svelte 5)

State as of 2026-10-12. Onboarding, the Microphone and Mixer screens, calibration, the TEST (A/B) panel
and Settings are ready. Preset import/export through a file dialog is the next step. Mixer —
[mixer.md](mixer.md).

The mock transport (`src/lib/ipc/mock.ts`) handles the connection, microphone, presets, calibration and
A/B. `src/lib/ipc/mock/` holds the device and application data (`fixtures`), pure functions
(`helpers`), synthetic signals (`signals`) and a separate mixer (`mixer`). The mixer owns the channels,
applications and meters, uses the generated IPC types and hands out copies of its state for replies
and events.

## Running

```sh
pnpm -C apps/macafon-ui install
pnpm -C apps/macafon-ui dev            # browser, mock daemon: http://127.0.0.1:1420
pnpm -C apps/macafon-ui tauri dev      # Tauri window with the real daemon
pnpm -C apps/macafon-ui tauri build --no-bundle   # target/release/macafon-ui
```

`macafon` without arguments starts `macafon-ui` (next to the binary or from `PATH`) — this is how the
tray (BG-004) and the launcher open it. A second launch focuses the already open window (BG-006,
`tauri-plugin-single-instance`). Closing the window ends the UI process; audio stays in the daemon (BG-001).

On the Linux development machine there are no system WebKitGTK/GTK packages: the headers and `.so` files
live in the same user-local sysroot as PipeWire (`. ~/.cache/macafon-sysroot/env.sh`). Building and tests
work; the window cannot be started there — WebKit looks for its processes at the hardcoded path
`/usr/lib/.../webkit2gtk-4.1`, and user namespaces for overriding the path are not allowed. The UI is
checked in a browser against the mock daemon, the bridge by tests (below).

## Structure

```
Svelte (runes) ──DaemonClient──► Transport ─┬─ TauriTransport ─invoke/event─► src-tauri/bridge.rs ─► macafond.sock / named pipe
                                            └─ MockTransport (browser; ?daemon=down — daemon unavailable,
                                                              ?mic=disconnected — microphone disconnected,
                                                              ?config=recovered — config reset,
                                                              ?overload — Studio → Lite,
                                                              ?platform=windows[&cable=missing] — Windows)
```

- `src/lib/ipc/protocol.ts` and `dsp.ts` are **generated** from Rust (`ts-rs` behind the `ts` feature,
  off by default — the daemon does not pull it in): protocol types, ranges and defaults from
  `params.rs`, macro curves, EQ characters and built-in presets. Do not edit by hand:
  `pnpm -C apps/macafon-ui gen:ipc`. `cargo test -p macafon-ipc --features ts` fails if the files are stale.
- `client.ts`: a typed `call(method, params)`; the result table is checked for completeness against the
  generated `Request` — a new protocol method will not compile until it is added.
- `state/daemon.svelte.ts`: a mirror of the daemon state. A chain edit is applied optimistically locally,
  then `mic.set`; while a request is in flight, new edits accumulate and only the last one is sent (never
  more than one request at a time). The `mic.changed` echo during our own edits does not overwrite the
  slider. Undo/Redo (DSP-008): edits closer than 600 ms are one step, depth 100, mute/bypass are not undone.
- Meters (MTR-004, NFR-R7): a 60 Hz subscription only while the window is visible (`visibilitychange`).
  Frames live outside reactivity (`state/meters.ts`); canvas meters are drawn in one shared
  `requestAnimationFrame`. Ballistics from the design system: 1 s peak hold, 20 dB/s fall, clip sticks for 2 s.
- `bridge.rs`: one thread owns the connection — the `hello` handshake, replies are matched by `id`,
  events and status go to the WebView (`daemon://event`, `daemon://status`). If there is no socket, it
  starts `macafond run` once (next to the binary or from `PATH`, in a separate process group so that the
  daemon outlives the UI; without a console window on Windows) and waits 4 s (§9.7). A lost connection
  does not restart the daemon (after Quit from the tray it must stay off; a crashed one is brought back by
  systemd) — after 4 s an "offline" screen with Retry. `Quit Macafon` in Settings stops the daemon and
  closes the UI (BG-005).

## Onboarding (§9.2)

A full-screen flow instead of the main window while the WebView's `localStorage` has no
`macafon.onboarding=done`. Six steps: Welcome → Microphone → Calibrate → Result → Use it → Background
(on Windows an Install VB-CABLE step follows Welcome); "Skip setup" in the header ends the flow without
side effects, Settings → Setup guide starts it again.

- Microphone: live meters of all inputs (`devices.subscribe` only while the step is open), the list is
  refreshed every 2 s. Until the user chooses, the input that has heard speech the longest
  (above −45 dBFS, 0.5 s) is selected; `mic.set_device` only on Next.
- Calibrate/Result: the same `CalibrationProgress` / `CalibrationResult` as in the dialog. Result embeds
  the TEST panel; "Listen A/B" records a clip, "Fine-tune" leads through steps 5–6 and after Done opens
  the Microphone screen. Without calibration the Result step is skipped.
- Use it: the system default toggle (`mic.set_default_source` with `confirmed: true` / `restore`),
  hints for Discord, OBS, Zoom (MIC-015).
- Background: "Start with system" is on by default and applied only on Done (AS-001).

## Clarity for beginners

- A "Getting started" block on the Microphone screen: calibration (marked as done), recording and
  comparing, selecting "Macafon Microphone" in applications + turning off their noise suppression
  (US-2/3/5, MIC-015). Hidden with "Got it" (a flag in the WebView's `localStorage`).
- Until calibration has been done, the Calibrate button is the primary (inverted) one. "Listen A/B" in
  the calibration result starts recording in the TEST panel right away.
- State banners: microphone not selected, disconnected (MIC-005), processing off (Bypass) with a
  "Turn on" button, input clipping.
- Visible captions without jargon: Before/After instead of Raw/Processed, Match volume, Cut and Noise
  instead of GR/NS, Mic instead of RAW; meters, Bypass, Mute and Latency have one-line hover hints.
  The CLIP indicator appears only on clipping.
- Save in the header is visible only when the preset has changed.

## Mixer screen (§9.4)

- A channel column: a color bar, application chips, a vertical fader (arrows ±1%, PageUp/Down ±10%,
  double click — 100%) with a stereo meter, % and dB, `M` / `EQ●` / `FX●`. Side panel: EQ with presets
  by category, FX (channel Bypass, NS, Compressor Amount, Limiter Ceiling), channel settings. On Windows
  there are no EQ/FX: a Channel button instead (WIN-CH-006).
- Moving an application: HTML5 drag & drop (a chip tilted 2°, the target is highlighted with its color
  and ticks), keys 1…8 on a focused chip, a context menu (Move to, Don't manage, Forget).
- Channel edits are optimistic and merged (like the microphone): never more than one `channel.set` per
  channel at a time. `mixer.meters` meters come with `meters` on the same subscription.
- Mock: `?screen=mixer`, `?mixer=off` — the mixer is not enabled yet.
- Quit in Settings asks for confirmation.

## Macros and CUSTOM (DSP-002)

`ChainSettings` has no main slider position — the UI derives it from the parameters by inverting the
`macafon_dsp::settings::macros` curves (they are exported to `dsp.ts`). If the parameters match no slider
position, the module is `CUSTOM`, the slider is locked, and "Reset to simple" is offered.

| Module | Main slider | How it is determined |
|---|---|---|
| Gain, High-pass, NS, Limiter | The parameter itself (gain, frequency, intensity, ceiling) | Always simple mode |
| Expander | Sensitivity | By ratio; range and times must match. **The threshold is not compared**: calibration sets it from the noise floor (CAL-003), and the module must not become CUSTOM because of that. Moving the slider sets the threshold relative to the floor after Gain (calibrated floor + gain) or relative to −56 dBFS without calibration |
| Compressor | Amount | By threshold; ratio, attack, release, knee, auto makeup, RMS must match |
| De-esser | Intensity | By threshold; frequency, max reduction, split-band. Listen is ignored |
| EQ | Character + Amount | The 5 bands of the default layout with the character's gains × Amount; any band edit makes it CUSTOM |

`macros.test.ts` checks the inverse mapping on the built-in presets from Rust — if the TS mirror diverges
from `from_amount`/`from_sensitivity`, the test fails.

## Deviations and gaps

- No preset import/export (P1) in the UI — it needs a file dialog (`tauri-plugin-dialog`); the CLI has it.
- BG-001: the hint on the first window close is shown by the daemon as a system notification (see mixer.md).

## Checks

```sh
pnpm -C apps/macafon-ui check                       # svelte-check + strict TypeScript
pnpm -C apps/macafon-ui test                        # Vitest: macros against presets, EQ response
cargo test -p macafon-ipc --features ts             # TS files are up to date
cargo test -p macafon-ui                            # the bridge against a fake daemon
pnpm -C apps/macafon-ui screenshots ../../.artifacts/screenshots [scenario…]   # with dev running
```

The bridge test starts a fake daemon on a Unix socket (a named pipe on Windows): replies out of order, an
event between them, a daemon error, the daemon disappearing with a pending call, the switch to
`unavailable` without starting a second daemon. The bridge was also checked once against a real
`macafond` in a private PipeWire: handshake, calls, 60 meter frames in 2 s at 30 Hz, error mapping,
`daemon.quit` → `unavailable`.
