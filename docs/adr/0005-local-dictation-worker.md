# ADR-0005: Local dictation in an on-demand worker process with two engines

- **Status:** accepted
- **Date:** 2026-10-19
- **PRD context:** [§8.4](../PRD.md#84-dictation-speech-to-text), [§19.2](../PRD.md#192-dictation-plan-stt)

## Context

The owner asked for Wispr Flow-like dictation that is free and fully local, with good English,
Ukrainian and Russian. Speech models need 0.2–1.3 GB of RAM and several CPU cores for a few seconds
per utterance, while the daemon must stay small (NFR-R4) and must not hurt games. The owner chose to
ship both candidate engines instead of benchmarking them first, and authorized model downloads from
Hugging Face.

## Options considered

| Option | Outcome |
|---|---|
| Inference inside `macafond` | Rejected: the daemon's RAM would grow by up to 1.3 GB, a native crash in an inference library would take audio down, and priority could not be lowered for inference alone |
| Inference in the UI process | Rejected: the UI may be closed (BG-001) and the WebView process is not meant to run heavy work |
| A worker that stays resident | Rejected: idle cost while gaming |
| **An on-demand worker process (`macafon-stt`)** | **Chosen** |
| One engine (Whisper only) | Rejected by the owner: Parakeet is several times faster on CPU |
| Models in the installer | Rejected: ≈ 0.2–0.65 GB each; NFR-R6 |

## Decision

Dictation runs in `apps/macafon-stt`, spawned by the daemon on the key press and exiting after an
unload timeout. The daemon owns the audio tap (lock-free, gated by an atomic), resampling, the
language policy, text insertion and the popup; the worker owns only inference behind a
`SpeechEngine` trait in `crates/macafon-stt` with two implementations: whisper.cpp (`whisper-rs`)
and Parakeet TDT 0.6B v3 on sherpa-onnx/ONNX Runtime. They talk over anonymous pipes; the worker
lives in a Job object that kills it with the daemon, runs at below-normal priority with EcoQoS and a
thread limit, and never touches the network. Models are downloaded by the daemon only on an explicit
user action, from Hugging Face URLs pinned to a commit and verified by SHA-256.

## Consequences

**Pros**
- Zero cost when unused: no process, no model in memory, one atomic load in the RT path.
- A crash or out-of-memory in inference never affects audio.
- The OS scheduler can deprioritize inference independently of the daemon.
- A third engine is one more `SpeechEngine` implementation.

**Cons / limitations**
- A cold start loads the model (0.5–2 s), mostly hidden behind speech time.
- Two native runtimes to build and ship; the installer grows by an estimated 15–25 MB (PRD §17 Q9).
- A second network endpoint besides the update check (Hugging Face), opt-in per download.
- Parakeet cannot be forced to a language, so the layout-based policy applies to Whisper only.

## When to revisit

If measurements in D1/D2 show one engine is clearly better on all three languages, if installer size
becomes a problem, or if a GPU backend becomes the default.
