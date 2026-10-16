# ADR-0001: Windows microphone processing through an APO plugin, without our own driver

- **Status:** superseded by [ADR-0004](0004-windows-free-vb-cable.md) (2026-10-11)
- **Date:** 2026-10-08
- **PRD context:** [§12.2](../PRD.md#122-windows-m3)

## Context

On Windows a virtual audio device (like `SteelSeries Sonar - Microphone`) requires a kernel-mode driver.
For it to load on ordinary systems it must be signed by Microsoft: an EV certificate (paid yearly) and
attestation signing in Microsoft Partner Center. Test-signing is not an option: a desktop watermark and
incompatibility with anti-cheats (Vanguard, FACEIT, etc.), and gamers are the core audience.

## Options considered

| Option | Cost | Outcome |
|---|---|---|
| Our own signed driver | EV certificate + Partner Center, a long process | Postponed until there is a budget |
| Test-signing | 0 | Rejected: anti-cheats, watermark |
| Bundling VB-Cable | VB-Audio redistribution license | Rejected for the microphone: foreign names, paid bundling |
| VB-Cable installed by the user | 0 for us | Chosen for output channels ([ADR-0002](0002-windows-output-vbcable.md)) |
| Open-source VirtualDrivers/Virtual-Audio-Driver | 0 | Rejected: currently requires test-signing |
| **APO plugin on the physical microphone** | 0 (Authenticode signing through SignPath Foundation) | **Chosen** |

## Decision

Microphone processing on Windows is done by an APO plugin (a user-mode COM DLL in `audiodg.exe`) that
is registered on capture endpoints and wraps the vendor APOs. The `macafond` daemon controls the
parameters through shared memory; the `macafon-apo-helper` service registers the APO on new devices and
repairs the registration after updates.

## Consequences

**Pros**
- No driver signing costs, no test-signing.
- Minimal latency: processing happens in the existing Windows audio engine cycle, without an extra pass through a virtual device.
- Processed sound in every application at once, nothing to switch in Discord/OBS.
- DSP code (`macafon-dsp`) shared with Linux.

**Cons / limitations**
- No separate "Macafon Microphone" device: different applications cannot get different processing.
- Administrator rights are needed at install time; updates can reset the registration (mitigated by the helper service).
- Possible conflicts with vendor APOs.
- An APO does not solve per-application output channels — those use VB-Cable ([ADR-0002](0002-windows-output-vbcable.md)).

## When to revisit

If a budget for an EV certificate and attestation signing appears, or if the APO model turns out to be
unstable on a noticeable share of devices after M3.
