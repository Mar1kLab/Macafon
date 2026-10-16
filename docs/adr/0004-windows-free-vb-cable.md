# ADR-0004: Windows only on the free VB-CABLE: the cable for the virtual microphone, channels as volume groups

- **Status:** accepted
- **Date:** 2026-10-11
- **PRD context:** [§12.2](../PRD.md#122-windows-m3), plan — [§19.1](../PRD.md#191-windows-port-plan-m3)
- **Supersedes:** [ADR-0001](0001-windows-apo.md) (APO plugin), [ADR-0002](0002-windows-output-vbcable.md) (channels on VB-Audio cables)

## Context

The owner decided that the Windows version uses the VB-Audio driver wherever a virtual device is needed,
and must be designed for the **free** version — VB-CABLE with a single cable (`CABLE Input` →
`CABLE Output`). The A+B and C+D cables are only provided after a donation. There is still no driver of
our own: signing requires an EV certificate and attestation, and test-signing is incompatible with
anti-cheats.

One cable carries audio in one direction. If the processed microphone and applications write into it at
the same time, the audio is mixed, so the cable can serve only one function.

## Options considered

| Option | Outcome |
|---|---|
| **A. The cable for the virtual microphone; channels as audio session volume groups without effects** | **Chosen**: the microphone is the product's main feature, and on Windows it works as on Linux (a separate device, A/B and calibration from the daemon) |
| B. The cable for the mixer (Master EQ/FX); the microphone through an APO | Rejected: an APO is complex (COM in `audiodg.exe`, a repair service, anti-cheats), and Master EQ is less valuable to the user than a processed microphone |
| C. The user chooses the cable's purpose in onboarding | Rejected: two implementation and test branches, confusing UI |
| D. Require A+B / C+D for channels (ADR-0002) | Rejected: contradicts the requirement to work on the free version |
| E. An APO plugin for the microphone (ADR-0001) | Rejected by the owner: VB-CABLE only |

## Decision

- The daemon captures the physical microphone through WASAPI, processes it with the `macafon-dsp` chain
  and plays it into `CABLE Input`; applications select `CABLE Output` — this is the "Macafon Microphone".
  The clock drift between the microphone and the cable is compensated by adaptive resampling.
  Requirements WIN-MIC-001…010.
- The Windows mixer has channels as application groups: the volume and mute of their audio sessions
  (`ISimpleAudioVolume`), session meters, up to 8 channels. No effects on channels or Master.
  Requirements WIN-CH-001…010.
- The user installs VB-CABLE from the official site; Macafon neither redistributes nor downloads it.
- No APO, no helper service, no administrator rights: a per-user installer (implemented as a per-user
  NSIS installer, see [windows-m3.md](../development/windows-m3.md)).

## Consequences

**Pros**
- The processed microphone is a separate device, as on Linux and in Sonar; shared chain, A/B and calibration code.
- Simple installation without system components; no conflicts with vendor APOs and no anti-cheat risks
  from a DLL in `audiodg.exe`.
- Works on the free version of VB-CABLE.

**Cons / limitations**
- Higher microphone latency than on Linux: two WASAPI passes and the cable buffer (NFR-L6, measured in M3.0).
- One virtual microphone: M4 (several microphones, Stream mix) is not available on Windows.
- No channel or Master EQ/FX on Windows.
- Without Macafon running, `CABLE Output` is silent; the system device name is foreign.
- The cable may be taken by other programs (OBS, VoiceMeeter) — Doctor has to watch for it.
- Dependency on a third-party driver.

## When to revisit

- After M3.0, if the VB-CABLE latency does not fit NFR-L6 with a reasonable buffer.
- If a way appears to provide per-channel effects without extra cables (for example, WASAPI process
  loopback with predictable session volume) — WIN-CH-010.
- If a budget for our own signed driver appears — several virtual microphones and channels with effects,
  as on Linux.
