# M1: microphone processing chain

State as of 2026-10-11. The chain is connected to the daemon, IPC, config, calibration and A/B.
The visual part of the UI did not change in this round.

## What exists

- Eight modules in the fixed PRD §6.3 order: Gain → High-pass → NS Lite/Studio → Expander/Gate →
  EQ → Compressor → De-esser → Limiter → Output gain. A disabled module is not created
  (0 CPU, 0 latency, DSP-005). Global bypass (MIC-014) and mute (MIC-013).
- Ranges and defaults live in `macafon_dsp::params`, the single source for CLI/UI/config.
- Filters are TPT SVF (Simper); responses match the RBJ prototypes after the bilinear transform
  (test ±0.1 dB, EQ-003). HPF — Butterworth 12/24 dB/oct.
- `MicrophoneProcessor::retune` changes continuous parameters and mute without allocations,
  with 10 ms smoothing (DSP-003). Enabling/disabling modules, bypass, the NS engine and the
  limiter look-ahead change the topology or latency — `RebuildRequired` is returned and a new
  processor is built outside the RT thread (NFR-S2).
- Lock-free meters (`Telemetry`): peak before Gain (input clip, MIC-G1), input and output peak/RMS,
  gain reduction of the expander/compressor/de-esser/limiter, NS activity (MTR-001…003, DSP-004).
- Six built-in presets from §6.3.10; in the daemon `macafond run --preset <name>`, Natural by default.

## Decisions not in the PRD

| What | Decision |
|---|---|
| Expander, Sensitivity macro | Threshold = noise floor + 30 dB (0%) → +6 dB (50%) → floor (100%). Ratio 1:1.5 → 1:3 → 1:6, Range −15 → −30 → −50 dB. Without calibration the noise floor is −56 dBFS, so 50% gives the −50 dBFS threshold from the table |
| Expander, hysteresis/hold | Opens at the threshold, closes below `threshold − hysteresis` after hold |
| Compressor, Amount macro | Threshold −10 → −35 dBFS linearly; Ratio piecewise linear 2:1 → 3:1 (40%) → 6:1, so that 40% matches the table defaults |
| Compressor, auto makeup | Compensates the static attenuation at a −18 dBFS speech level (calibration target CAL-003) |
| Compressor, RMS detector | 10 ms window. Auto release adapts to the attenuation duration |
| De-esser, Intensity | Threshold −10 dBFS (0%) → −50 dBFS (100%), ratio 4:1, attack 0.5 ms, release 40 ms |
| De-esser, split-band | A dynamic bell on the same band as the detector (band-pass, Q 1): at the center frequency the attenuation equals the gain reduction exactly, far away it is 1 |
| Limiter without look-ahead | Attack 0.2 ms, attenuation down to `ceiling − 0.5 dB`, then a soft tanh clipper: the output never exceeds the ceiling |
| EQ characters (dB at Amount 100%) | Bands LS 120 / 250 / 1k / 4k / HS 10k: Clarity −2/−3/0/+3/+2, Warmth +3/+1.5/0/−1/−2, Broadcast +2/−2/0/+2/+3, Presence 0/−1/+1/+4/+1, Thin mic fix +4/+2/0/−1/0 |
| Studio NS | Embedded DeepFilterNet3 model, a separate worker with fixed SPSC frames. On overload — switch to Lite and an IPC event; the model is never loaded from the network |

Macro percentages are stored in the 0–100 range, as in the PRD tables; the config example in PRD §13
uses 0–1 fractions — the conversion will live in `macafon-config`.

## Notes on the PRD

- §6.3.4: "Range and Ratio grow with strictness", but per the text 100% Sensitivity is a threshold at
  the noise floor (the softest mode), while the Noisy Room preset sets 70%. Currently the threshold
  goes down while ratio/range go up with the slider. The slider direction needs a clear definition.
- §11.4: the chain is described as `Vec<Box<dyn Processor>>`. The order is fixed in 1.0, so modules
  are stored as `Option` fields: no virtual calls and no allocations when disabled. Reordering modules
  (DSP-009, P2) will require a rework.

## Performance

`cargo bench -p macafon-dsp --bench microphone`, release, 128-sample block (sine generation
~0.57 µs):

| Chain | Time per block | Share of one core at 48 kHz |
|---|---|---|
| Raw | 0.57 µs | — |
| Broadcast without NS | 9.11 µs | ≈ 0.34% |
| Natural (with NS Lite) | 48.9 µs | ≈ 1.83% |
| Auto release, without NS | 9.23 µs | ≈ 0.35% |
| Meters, without NS | 9.88 µs | ≈ 0.37% |

This is a DSP micro-benchmark on a local machine, not a confirmation of NFR-R1…R3 for the whole daemon.

Envelopes "snap" to the target instead of decaying exponentially into denormals: without FTZ
(tests, benchmarks) the expander was otherwise 6× slower. In the RT thread PipeWire enables FTZ/DAZ
anyway.

## Checks

```sh
cargo test -p macafon-dsp
cargo bench -p macafon-dsp --bench microphone -- --sample-size 10 --measurement-time 1 --warm-up-time 1
python3 tools/tests/pipewire-smoke.py   # includes a run of the Natural preset through the graph
```

## M1 additions

Output telemetry includes short-term LUFS (K-weighting, 3 s window), the held peak and clip
latches. DSP/engine test the RT pass under `assert_no_alloc`. Three audio fixtures with real speech
and noise check Natural, Broadcast and Noisy Room against stored output samples. This is a
computation regression, not a listening-quality assessment.

Lite latency: 960 samples (20 ms) before the graph; the NFR-L2 target is not met yet.
Studio: 2400 samples (50 ms), including the model and the worker adapter. Disabled NS — 0.
Physical loopback, subjective listening and suspend/resume need a separate test machine.
