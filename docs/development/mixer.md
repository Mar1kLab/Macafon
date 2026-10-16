# M2: mixer (output channels)

State as of 2026-10-11. Channels, applications and outputs (PRD §7) are ready in the daemon, CLI and UI;
Doctor (§9.6); hotkeys (§8.3) in the daemon and CLI — the Hotkeys section in the UI is next.

## Design

[ADR-0003](../adr/0003-linux-channels-pw-filter-sinks.md): a channel is a `pw_filter` node
`Audio/Sink` `macafon_ch_<id>` (description "Macafon <Name>"), stereo. Applications connect to the
inputs, the chain processes the buffer in the same cycle, the daemon links the outputs to the real
device. Master is a volume multiplier inside every channel.

| Layer | Where |
|---|---|
| Chain `EQ → NS Lite (L/R) → Compressor → Limiter → Volume`, stereo-linked dynamics, CFX-005/006 templates, CFX-002 EQ presets, CH-004 cubic curve | `macafon-dsp/src/channel.rs` |
| Lock-free control, crossfade on rebuild | `macafon-engine/src/channel.rs` |
| Application key (APP-002), name, known-application database (APP-006), system sounds (APP-011) | `macafon-apps` |
| Channel nodes, links to the output, stream tracking, `target.object` | `macafon-backend-pipewire/src/{channel_node,mixer}.rs` |
| Decisions: which channel, which output, default output | `macafond/src/mixer.rs` |
| Config `[mixer]`, `[[mixer.channels]]`, `[[apps.rules]]` | `macafon-config/src/mixer.rs` |

## Behavior

- The mixer is off by default (CH-009); it is turned on in onboarding (Mixer step), on the Mixer
  screen, in the CLI.
- When on: the system default output = the default channel (OUT-005). The previous output is written
  to the journal `$XDG_STATE_HOME/macafon/default-sink.json` and given back on disable, Quit, crash (on
  the next start) and `macafon restore-default`. After a PipeWire restart the journal keeps the
  original output.
- An application's channel: rule → built-in classification (if enabled) → the default channel. Moving a
  chip writes a rule and `target.object`; the stream is not restarted (APP-004/005). A deleted channel →
  its rules move to the default channel.
- Output: the first present device from the priority list → the previous system output → the current
  system output if it is a real device → the first device. Connect/disconnect is handled within ≤ 100 ms
  (OUT-002).
- Streams with `node.dont-move` / `node.dont-reconnect` are left alone; the chip shows a lock.
- Our own streams (`macafon_*`, the A/B player) are not routed; with the mixer on, A/B plays straight
  into the output device, not into a channel.

## CLI (HK-004)

```sh
macafon ctl mixer on|off|toggle          # no argument — state
macafon ctl channel voice volume +5      # -5, or an absolute value: 40
macafon ctl channel main mute [on|off|toggle]
macafon ctl channel music default
macafon ctl apps [list] | macafon ctl apps move bin:firefox music
macafon ctl output <node.name>|default
```

## Checks

`python3 tools/tests/mixer-smoke.py` — private D-Bus/PipeWire/WirePlumber: channel nodes and their links,
the default output and its restore, Discord → Voice classification, moving without a restart and the rule
on a second launch, audio on the output device, CLI, output change, priority on device connect and
disconnect, disabling the mixer, a PipeWire restart, Quit without leftover nodes.

## Deviations and gaps

- The Master meter is the maximum of channel peaks after Master, not a device measurement.
- A rule is applied within ≤ 100 ms after a stream appears: a short sound in the default channel is possible.

## Tray and notifications

Tray menu: a Mixer toggle and `Output device ▸` (BG-003). On the first window close the daemon shows the
freedesktop notification "Macafon keeps running in the tray" (BG-001, flag `[ui] close_hint_shown`).
Check — `tools/tests/tray-smoke.py` (dbus-monitor in a private session).

## Doctor (§9.6)

`doctor.run` / `macafon ctl doctor`: PipeWire and WirePlumber versions (a graph snapshot over a separate
connection), microphone selected/connected, whether Macafon Microphone is in use, third-party noise
suppressors (EasyEffects, NoiseTorch, RNNoise filters), channels linked to the output, unmovable
applications, quantum and Studio → Lite fallbacks in the last 10 minutes. UI: Settings → Doctor with
one-click fixes (select a microphone, make it default, enable the mixer) and copying a report without
device and application names.
The `xruns` check: on its own connection Doctor subscribes to the PipeWire profiler for ≤ 300 ms and takes
the `SPA_PROFILER_info` of the driver that runs Macafon Microphone: DSP load (the "medium" average) and the
xrun counter. Deviation from §9.6: the counter is since the driver started, not for the last 10 minutes;
without active microphone processing or without `module-profiler` on the server — "not measured".

## Hotkeys (§8.3)

Registered by the daemon (HK-003): the `org.freedesktop.portal.GlobalShortcuts` portal through zbus (a new
session on every change; the desktop may assign other keys — `system_trigger`), otherwise in an X11 session
`global-hotkey` (x11rb), otherwise `unavailable` and CLI commands (HK-004). HK-001 actions: mute,
push-to-talk, push-to-mute, bypass, next/previous preset, volume ±5% and mute of each channel. Config
`[hotkeys]`: `mic_mute_toggle = "Ctrl+Alt+M"`, per channel — `channel_volume_up.voice`. Shortcuts are
normalized, duplicates within Macafon are rejected (HK-002). OSD notification on preset and volume change
(HK-006, one notification replaces the previous one).

```sh
macafon ctl hotkeys [list] | macafon ctl hotkeys set push_to_talk F9 | … set <action> none
macafon ctl mic mute toggle | macafon ctl preset next|previous
```

Check — `tools/tests/hotkeys-smoke.py` with the fake portal `tools/tests/fake_portal.py` (python-dbus).
Not checked: real KDE/GNOME/Hyprland portals, X11 key grabs (no XTEST on the test machine; build only).

Settings → Hotkeys (HK-002): actions by group (microphone, each channel), recording a shortcut by pressing
it (physical keys `KeyboardEvent.code`, the same canonical form as `Accel`), a conflict within Macafon is
shown before the request to the daemon, the portal's `system_trigger` as a caption. Without a portal and
X11, CLI commands with copy buttons are shown instead of recording (HK-004).

## Application volume (CH-008)

`apps.set_volume { app_key, volume_pct }` is remembered in the rule (`volume_pct` in `[[apps.rules]]`, the
same cubic curve as the channel fader). The daemon writes `SPA_PROP_volume` into the Props of each of the
application's stream nodes — a separate multiplier; `channelVolumes` stay with the application and
pipewire-pulse. "Don't manage" applications, a disabled mixer and Quit restore the multiplier to 1.0.
UI: clicking a chip opens a mini slider; anything other than 100% is shown on the chip.

## Application icons (APP-003)

`apps.icon { icon }` → a `data:` URL or `null`. The resolver in `macafon-apps::icons`: `.desktop` files
(by file name, the binary from `Exec=`, `StartupWMClass`) → `Icon=` → the hicolor theme, then other themes
(`<size>/apps` and `apps/<size>`), then `pixmaps`; SVG/PNG up to 512 KiB. Directories — `XDG_DATA_HOME`,
`XDG_DATA_DIRS`, Flatpak exports. Found icons are cached until restart, misses for 60 s.
The UI caches the reply by name and draws a letter until then. On Windows an icon key is an absolute
exe path, and the icon is extracted from the executable (see [windows-m3.md](windows-m3.md)).
