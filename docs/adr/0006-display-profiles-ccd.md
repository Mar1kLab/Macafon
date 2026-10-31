# ADR-0006: Display profiles over the Windows CCD API with stable monitor identity

- **Status:** accepted
- **Date:** 2026-10-21
- **PRD context:** [§8.5](../PRD.md#85-display-profiles), [§9.10](../PRD.md#910-displays-screen), [§19.3](../PRD.md#193-display-profiles-plan)

## Context

The owner asked for quick switching between saved monitor setups (which monitors are on, primary,
resolution, refresh rate, layout) by hotkey and from the interface, as a Windows feature outside the
milestone order (2026-10-21). Existing tools such as MonitorSwitcher store raw adapter LUIDs and
target ids in their profiles; those change after a reboot, a driver update or a port change, and the
profile then silently fails. They also have no confirmation: a profile with a mode the monitor or
cable cannot do leaves a black screen.

## Options considered

| Option | Outcome |
|---|---|
| `ChangeDisplaySettingsEx` per GDI device | Rejected: cannot turn monitors on, cannot set duplicate mode or the primary reliably together, no validation of a whole layout |
| **CCD: `QueryDisplayConfig` / `SetDisplayConfig`** | **Chosen**: the API Windows Settings uses; whole-layout validation with `SDC_VALIDATE`; documented HDR control through `DisplayConfigSetDeviceInfo` |
| Storing raw CCD path and mode arrays as the profile | Rejected: LUIDs and target ids are not stable |
| Per-monitor DPI scaling in profiles | Rejected by the owner: only an undocumented API exists |
| A separate helper process | Rejected: the calls are short and need no isolation; the daemon already runs at login |

## Decision

`crates/macafon-backend-windows/src/displays.rs` reads and applies layouts through CCD. A profile
stores, per monitor, a `DisplayIdentity` (EDID manufacturer and product code, EDID serial when known,
device path) and a layout (position, mode, rotation, primary, mirror source, HDR). Each apply resolves
the identities against the connected monitors — path first, then EDID with serial, then an EDID only
one connected monitor has — and builds a fresh configuration; monitors not in the profile are turned
off, missing ones are skipped and reported. The layout is validated with `SDC_VALIDATE` before it is
applied. The daemon snapshots the raw configuration before every apply: on a set of monitors the
profile was never kept on, a "Keep these display settings?" countdown restores the snapshot after
15 s unless kept, and "Previous layout" restores it on demand. Profiles change system display
settings on purpose and persist after Macafon quits; nothing is restored on quit or uninstall.

## Consequences

**Pros**
- Profiles survive reboots, driver updates and port changes.
- No administrator rights; no black screen without a way back (validation, countdown, the
  "All monitors on" hotkey).
- Hotkeys, the tray and autostart come from the existing daemon.

**Cons / limitations**
- Two identical monitors without EDID serials are told apart only by port; after swapping ports
  their layouts swap.
- Some drivers accept a layout in validation and still fail to light a monitor; the countdown
  covers it.
- Linux compositors have no common API; their separate adapters have different optional capabilities.

### Linux (2026-10-31)

The owner requested Linux parity. `macafon-displays-linux` is separate from the audio backend and
from CCD; all its code and dependencies are Linux-only. A successful read of the running desktop
selects Mutter DisplayConfig, KScreen, wlr-randr or native XRandR, in that order (KScreen is probed
in Plasma; XRandR is excluded on Wayland). The daemon overrides only the Linux hello's display
capability; the static fallback stays false and Windows capabilities stay unchanged. Backend
selection is cached until the daemon restarts.

Profiles retain ADR-0006 identity precedence. Connector-matched `/sys/class/drm/*/edid` supplies
manufacturer/product/serial where available; ambiguous DRM connectors are not guessed. Mutter's
vendor/product/serial and wlroots make/model/serial are fallbacks when raw EDID is inaccessible.
CLI commands use argv without a shell, fixed locale, bounded output and a 2 s timeout. Mutter uses
bounded blocking zbus calls. Display signals/polls and notification actions run on helper threads;
the existing control loop remains responsible for apply, rollback and the deadline.

Mutter method 1 previews the layout, method 0 verifies it, and method 2 requests persistence on Keep.
Mutter's current implementation requests GNOME Shell confirmation for method 2; there is no public
DisplayConfig method to answer it. A second Shell confirmation may therefore be needed to save
across logins, including on a previously confirmed profile. This is an explicit Linux limitation,
not an undocumented Shell API dependency. Rollback restores exact Mutter scale/transform/color
settings using a fresh serial. CLI snapshots retain the layout and existing scales. A freedesktop
notification supplies Keep / Revert and a countdown even without the window; missing notification
support falls back to the daemon's automatic rollback.

Optional fields are applied only where supported: primary on Mutter/KScreen/XRandR, mirroring on
Mutter/XRandR and KScreen versions advertising `.mirror.`, HDR on Mutter color modes/KScreen HDR.
wlroots has neither primary, mirroring nor HDR through this protocol. Scaling remains outside
profiles. CLI layouts are session changes; login persistence is desktop-dependent. Unsupported
compositors stay hidden by capability detection. Live desktop/hardware validation is still pending.

API references: [Mutter DisplayConfig](https://github.com/GNOME/mutter/blob/main/data/dbus-interfaces/org.gnome.Mutter.DisplayConfig.xml),
[Mutter confirmation handling](https://github.com/GNOME/mutter/blob/main/src/backends/meta-monitor-manager.c),
[KScreen doctor](https://invent.kde.org/plasma/libkscreen/-/tree/master/src/doctor),
[wlr-randr](https://gitlab.freedesktop.org/emersion/wlr-randr).

## When to revisit

If Windows documents per-monitor scaling, if users ask for automatic switching on dock connection
(DISP-011), or when Linux compositors expose additional primary/mirror/HDR or persistence APIs.
