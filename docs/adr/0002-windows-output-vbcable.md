# ADR-0002: Windows output channels through user-installed VB-Cable

- **Status:** superseded by [ADR-0004](0004-windows-free-vb-cable.md) (2026-10-11)
- **Date:** 2026-10-08
- **PRD context:** [§12.2.2](../PRD.md#1222-windows-mixer--volume-groups-without-cables) (the section has since been rewritten by ADR-0004)
- **Related:** [ADR-0001](0001-windows-apo.md) (microphone through an APO, no driver of our own)

## Context

Output channels (Main / Voice / Music) need output devices that applications see as ordinary speakers
and that Macafon can capture, process and mix. Our own driver is postponed (ADR-0001). An APO on an
output device only sees the final mix, so audio cannot be split per application through an APO.

## Options considered

| Option | Outcome |
|---|---|
| A. Process loopback (`AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`) + a "mute" device / zeroed session volume | Rejected: non-obvious session volume behavior, needs a "mute" physical device, fragile |
| **B. VB-Audio Virtual Cable cables installed by the user** | **Chosen** |
| C. Master EQ/FX only through an APO, no channels | Rejected as the main mode; kept as a reduced mode without cables |
| Bundling VB-Cable in the installer | Rejected for now: requires a paid VB-Audio redistribution license |

## Decision

Channels = VB-Audio cables (VB-Cable, A+B, C+D — up to 5 cables). The user installs them from the
official site (donationware); Macafon finds the cables by hardware ID, assigns applications to cables
through `IAudioPolicyConfig`, captures the cable outputs through WASAPI, processes them and plays them
into the real device. Requirements — WIN-CH-001…014 in the PRD at the time.

## Consequences

**Pros**
- Free for the project, no driver signing.
- VB-Cable is a signed, widely used driver, compatible with anti-cheats.
- The same "channel = device" model as on Linux and in Sonar.

**Cons / limitations**
- An extra installation step with a reboot — a risk of drop-off in onboarding.
- No more than 5 channels on Windows.
- Higher latency than on Linux (cable buffer + an extra WASAPI pass).
- Foreign device names (partly solved by renaming, WIN-CH-011 at the time).
- If Macafon is not running, nobody hears the sound in the cables — needs protection (WIN-CH-007 at the time).
- Dependency on a third-party project.

## When to revisit

- Low conversion of the VB-Cable installation step in onboarding → negotiate a bundling license.
- A budget for our own signed driver appears → virtual microphone and channels as on Linux.
