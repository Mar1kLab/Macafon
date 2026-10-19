# Dictation worker measurements - 2026-10-19

Scope: STT-008, STT-021, NFR-L7, NFR-R10. Windows 11, owner's i5-13600K
(6 P + 8 E cores). CPU inference; same round-one models and `en.wav` (9.94 s)
in `%TEMP%/macafon-stt-models`. Short input is the first 3 s of that recording.
Warm results discard the first inference and show the next two runs. Fresh-process
load times include process startup; the filesystem cache was warm. RAM is process
working set, not private bytes. These are worker timings, excluding daemon insertion.

## Findings

- Seven threads, fixed English, existing optimized release binary: EcoQoS plus
  BELOW_NORMAL took Turbo 20,841 / 16,752 ms and Parakeet 13,200 / 12,962 ms.
  Disabling only EcoQoS gave Turbo 11,913 / 10,315 ms and Parakeet 1,080 / 963 ms.
  Normal priority without EcoQoS gave Turbo 9,716 / 9,775 ms and Parakeet
  1,285 / 1,438 ms. Small was noisy: 2,536 / 2,708 ms with EcoQoS versus
  3,229 / 2,331 ms without; normal/no-EcoQoS was 2,202 / 2,121 ms.
  EcoQoS is a major Parakeet bottleneck; normal priority is not a consistent win.
- The original measured native release already used /O2, /MD (not /MDd), and
  /arch:AVX2 with GGML_AVX2/GGML_FMA/GGML_F16C. The apparently OFF cache values
  were overridden by GGML_NATIVE's local MSVC detection. The old packager-only
  flag workaround did not protect ordinary Cargo builds. The project-specific
  CMake hook now verifies /O2 /DNDEBUG and explicit AVX2/FMA/F16C with NATIVE OFF.
  No global Rust flags, host-native ISA, or new runtime dependency was introduced.
  Static whisper-rs does not ship ggml's dynamic CPU variants; AVX2 is the baseline.
- Parakeet uses the pinned int8 encoder/decoder/joiner, CPU provider, greedy_search,
  sherpa-onnx 1.13.8 and ONNX Runtime 1.28.2 from the matching Windows Release archive.
  Six recognition threads: six VAD threads measured 40 ms VAD + 861 ms recognition;
  one VAD thread measured 51 ms VAD + 383 ms recognition (901 -> 434 ms total).
  Separate multi-threaded runtime pools can affect recognition beyond VAD's own cost.
- Whisper greedy best-of-one and no_context=true were already set. At six threads,
  warm Turbo was 9,365 ms normally, 10,830 with flash attention, 9,661 with
  single_segment, and 13,878 with beam size 5. OpenMP measured 9,268 ms with
  six VAD threads and 9,193 ms with one: no material recognition gain; rejected.
- audio_ctx=500 reduced English Turbo to 2,628 ms and Small to 1,054 ms. This is
  not a safe default: on the existing 13.42 s Russian fixture Turbo at audio_ctx=750
  repeated the final sentence (4,292 ms), while full context was correct (9,413 ms).
  Full context remains enabled. Flash attention and beam search remain disabled.
- Automatic language detection adds another full encoder pass: final Turbo warm
  total is 18,258 / 18,360 ms versus fixed-English 9,240 / 9,185 ms.
  The restricted language-probability policy is preserved rather than skipped.

## Thread sweep

BELOW_NORMAL, no EcoQoS, fixed English; first inference discarded. Last warm run
shown below. Whisper VAD used the recognition budget in this sweep; final builds
use one VAD thread. Parakeet VAD used one thread throughout.

| Recognition threads | Turbo ms | Small ms | Parakeet ms |
|---|---:|---:|---:|
| 4 | 13,310 | 2,740 | 364 |
| 6 | 9,634 | 2,012 | 370 |
| 8 | 9,232 | 1,979 | 728 |
| 12 | 7,739 | 1,999 | 1,497 |

Four to six threads are preferable for Parakeet; increasing its budget hurts.
Whisper still misses NFR-L7 even at twelve threads with accurate full context.

## Final measured build

Six recognition threads, one VAD thread, BELOW_NORMAL, EcoQoS off; Whisper greedy,
no_context=true, full audio context, no flash attention. Fixed English for Whisper;
Parakeet always detects language itself. RAM shows loaded / peak working set.

| Model | Clip s | Load ms | Warm inference ms | RAM MiB loaded / peak |
|---|---:|---:|---:|---:|
| Turbo q5_0 | 9.94 | 386 | 9,240 / 9,185 | 569 / 964 |
| Turbo q5_0 | 3 | 384 | 9,109 / 9,387 | 569 / 963 |
| Small q5_1 | 9.94 | 146 | 2,168 / 2,263 | 203 / 495 |
| Small q5_1 | 3 | 165 | 2,106 / 2,098 | 203 / 494 |
| Parakeet int8 | 9.94 | 1,489 | 422 / 463 | 726 / 773 |
| Parakeet int8 | 3 | 1,560 | 147 / 141 | 726 / 750 |
| Turbo Auto | 9.94 | 413 | 18,258 / 18,360 | 569 / 965 |
| Turbo Auto | 3 | 383 | 19,628 / 18,781 | 569 / 963 |

Parakeet meets the worker portion of NFR-L7. Whisper does not. Short recordings
still pay for Whisper's padded full-context encoder. Further native optimization
or encoder reuse for language detection remains necessary; a shorter context is
not validated for production. Auto Small and remaining extended checks/measurements
were stopped at the owner's wrap-up request. No FPS test or disk-cold cache flush.

## Integration and reproduction

`WorkerRequest::Load` adds `efficiency: bool` with `#[serde(default)]` (false).
Ordinary dictation: false, BELOW_NORMAL, explicit EcoQoS disable. Game mode: true,
EcoQoS enabled, requested thread budget clamped then halved (minimum one), CPU
forced even when gpu=true. Ready.threads reports the effective count. The daemon
must derive this flag from SHQueryUserNotificationState at key press and reload
when it changes; it must also unload after a game-mode result. Do not send true
unconditionally or double the thread budget. The owner's policy supersedes the
older unconditional-EcoQoS wording of STT-008; updating the PRD is outside this scope.

Run `benchmark.py --phase scheduling|threads|tune|final|auto --output <jsonl>` with
Python on Windows. `--models` and `--exe` select fixtures and binary. `transcribe`
now also supports --repeat, --efficiency, --normal-priority, --vad-threads,
--flash-attention, --single-segment, --audio-ctx and --beam-size for local probes.
The engines expose stage timings without adding a library logging dependency.
Raw JSONL/native logs remain in `%TEMP%/stt-*.jsonl` and `%TEMP%/stt-*.log`.

A clean native rebuild was used to verify the Cargo hook. For an older cached
native target, use `cargo clean -p whisper-rs-sys` once. Standard development
prerequisites still apply: CMake and libclang (this session used the VS CMake
installation and the existing temporary libclang from round one). No installer
was rebuilt or installed during this follow-up.
