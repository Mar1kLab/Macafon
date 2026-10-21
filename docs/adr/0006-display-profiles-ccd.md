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
- Windows only; Linux compositors have no common API (`Capabilities::displays` is false there).

## When to revisit

If Windows documents per-monitor scaling, if users ask for automatic switching on dock connection
(DISP-011), or if Linux support is requested (KDE/GNOME/wlroots each need their own backend).
