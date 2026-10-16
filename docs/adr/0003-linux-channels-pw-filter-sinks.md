# ADR-0003: Linux output channels — our own `pw_filter` sink nodes

- **Status:** accepted
- **Date:** 2026-10-11
- **PRD context:** [§7](../PRD.md#7-functional-requirements--output-channels), [§12.1](../PRD.md#121-linux--pipewire-m0m2), NFR-L3
- **Related:** the M0 virtual microphone — the same `pw_filter` technique ([linux-m0.md](../development/linux-m0.md))

## Context

Each mixer channel must be a separate output device that applications see and to which WirePlumber can
move their stream through `target.object`. A channel's audio goes through the chain
`EQ → NS → Compressor → Limiter → Volume` and is mixed with the others into one real device.
PRD §12.1 left the choice to M0: an internal mix in one playback stream, or mixing done by PipeWire.
NFR-L3 requires that a channel without NS adds no buffering beyond the graph cycle.

## Options considered

| Option | Outcome |
|---|---|
| A. A pair per channel: an `Audio/Sink` capture stream + a playback stream to the device, joined by an SPSC buffer | Rejected: an extra quantum of latency per channel, two nodes, desynchronization on restarts |
| B. N sink nodes and one shared playback stream with an internal Master mix | Rejected: sink nodes and the output stream are not ordered in the graph cycle; the mix needs a 1-quantum buffer and state shared between callbacks |
| **C. One `pw_filter` per channel: inputs are `Audio/Sink` ports, Macafon links the outputs to the device itself** | **Chosen** |
| D. PipeWire modules (`filter-chain`, `loopback`) | Already rejected in PRD §12.1: config files, no meters or real-time parameters |

## Decision

A channel is a `pw_filter` node with `media.class = Audio/Sink`, `node.name = macafon_ch_<id>`, stereo
inputs `playback_FL/FR` and outputs `output_FL/FR`. Applications are linked to the inputs by WirePlumber
as to an ordinary sink; the chain processes the buffer in place in the same cycle; the daemon links the
outputs to the selected device through `link-factory` (like the virtual microphone input). PipeWire mixes
several channels into the device itself. Master is a shared volume multiplier inside every channel.
Channels with the `macafon_ch_` prefix are excluded from the output list (OUT-004) and from our own routes.

Application routing writes `target.object` (the channel node's serial) into the `default` metadata; the
daemon tracks `Stream/Output/Audio` streams through the registry and full `NodeInfo`. The system default
output (OUT-005) is taken over with the same journal scheme as the default source (MIC-012).

## Consequences

- A channel without NS adds 0 samples of buffering; the chain latency is reported through `ProcessLatency`
  (NFR-L4). Verified in a private PipeWire 1.6.2 + WirePlumber 0.5 graph: `tools/tests/mixer-smoke.py`.
- Channel links to the device are not passive: while the mixer is on, the device does not suspend. The
  `link.passive` variant was not tested; the impact on laptop power must be evaluated on a test machine.
  A disabled mixer creates no nodes at all (CH-009).
- The Master meter is the maximum of channel peaks after the Master fader, not a measurement on the device:
  application audio that bypasses Macafon (`node.dont-move`) is not visible in it.
- Streams with `node.dont-move` / `node.dont-reconnect` are not moved; the UI shows them as unmovable.
- A rule is applied within ≤ 100 ms after a stream appears (the control loop tick). Until then the stream
  may already be connected to the default channel — a short sound in the wrong channel is possible
  (see APP-005).
