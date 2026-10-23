# AGENTS.md — guidelines for agents and Macafon developers

Read this file before any work in the repository. On process questions it overrides the PRD: if the
PRD mentions something this file forbids, this file wins.

## 1. Project context

Macafon is a lightweight open-source Rust application: microphone processing (DSP chain + virtual
microphone) and a per-channel audio mixer with drag & drop of applications. Windows 11 is the working platform;
Linux (PipeWire) is experimental (owner decision, 2026-10-17).

| Document | Why read it |
|---|---|
| [docs/PRD.md](docs/PRD.md) | Requirements, requirement IDs (MIC-, DSP-, CH-, APP-, WIN-CH- …), parameter ranges, NFRs, architecture |
| [docs/design/design-system.md](docs/design/design-system.md) | Color tokens, fonts, UI components — the only source of styles |
| [docs/adr/](docs/adr/) | Accepted architecture decisions (Windows only on the free VB-CABLE — ADR-0004) |

See PRD §5 / §19 for the current stage. Do not run ahead: no Windows code before M3 and no M4+
features without a direct request, but keep things extensible where the PRD explicitly requires it
(for example, N virtual microphones).

## 2. Process

- **GitHub Actions scope.** The owner authorized a manual Windows 11 x64 installer build on
  2026-10-17 (`.github/workflows/windows-build.yml`). Keep other checks local; additional cloud CI,
  Linux workflows and automated release publishing need explicit owner authorization.
- **No bureaucracy.** No issue/PR templates, CHANGELOG ceremonies, extra READMEs in every folder or
  documents "for the future". Write an ADR only for real architecture decisions (choosing an approach
  that is hard to roll back) — short, following `docs/adr/0001-*.md`.
- **The PRD is the source of requirements.** If the implementation diverges from the PRD or a PRD
  decision looks wrong, do not silently change behavior: implement the PRD and raise the problem in
  the report, or ask before starting. Mention requirement IDs in the report (not in code).
- **Git.** Conventional Commits in English (`feat(dsp): add de-esser`, `fix(pw): …`, `docs: …`).
  Small, logical commits. Commit and push when the owner asks or the task says so.
  Do not rewrite published history.
- **Versions.** Every push to `main` that changes code (Rust, UI, packaging, build scripts) bumps
  the app version in the same push, as the last commit (`chore: release X.Y.Z`), with
  `python tools/bump-version.py patch|minor`:
  - **patch** `x.y.Z` — fixes, refactoring, performance, small UI edits;
  - **minor** `x.Y.0` — new features, new requirement IDs, large changes or protocol/config format
    changes;
  - **major** `X.0.0` — only on the owner's request.
  One bump per push (several commits in one push share it). Docs-only and agent-rule pushes do not
  bump. Do not edit versions by hand: the script keeps `Cargo.toml`, `Cargo.lock`, `package.json`
  and `tauri.conf.json` in sync.
- **Language.** Code, identifiers, comments, commits, UI strings and documentation in `docs/` — English.

## 3. Checks and tests — targeted, not "everything"

The goal is to quickly confirm that what changed works, not to run the whole world on every edit.

**By default (local changes):**
- `cargo fmt` and `cargo clippy -p <affected crate> -- -D warnings`.
- `cargo test -p <affected crate>` or a specific test (`cargo test -p macafon-dsp compressor`).
- Frontend: `pnpm -C apps/macafon-ui check` (svelte-check/TS) and only the relevant Vitest files.
- New DSP math or a fixed bug → **one targeted test** that checks exactly that
  (filter response against the formula, compressor static curve, bug regression).

**Only when justified:**
- `cargo test --workspace` — for changes in shared crates (`macafon-engine`, `macafon-ipc`,
  `macafon-config`, `macafon-backend`) or in APIs between crates.
- e2e (tauri-driver), soak tests, headless PipeWire integration, latency probe — for changes in the
  corresponding subsystem (PipeWire backend, RT path, routing) or when the owner asks. Do not write
  e2e tests for every UI edit.
- Benchmarks (`criterion`) — for changes in hot DSP code.

Do not add tests for coverage: no duplicate tests, trivial getters or huge snapshots.

## 4. Rust

- Cargo workspace (layout — PRD §11.2), stable toolchain, edition 2024.
- Errors: `thiserror` in libraries, `anyhow` in binaries. No `unwrap()` outside tests; `expect("why")`
  only for real invariants.
- Logging — `tracing`. `unsafe` only for FFI/COM/PipeWire, with a `// SAFETY:` comment.
- Keep dependencies minimal. Before adding a crate: is it needed, is it maintained, is the license
  compatible with an open-source project (no proprietary or network SDKs). `macafon-dsp` — no heavy
  dependencies.
- Units in names: `threshold_db`, `freq_hz`, `attack_ms`, `gain_lin`. Ranges and defaults strictly from
  the PRD §6.3 tables; keep them in one place (parameter descriptions in `macafon-dsp`), UI and config
  take them from there.

### 4.1 Real-time audio — hard rules

Breaking these rules means crackles and xruns, so they matter more than code convenience.

- In `process()` / data loop callbacks / WASAPI threads the following are **forbidden**: allocations
  (including `Vec::push`, `format!`, `Box::new`, `clone()` of owning types), mutexes and any waiting,
  system calls, logging, IPC, panics.
- All memory is allocated in `prepare()` / while building the chain outside the RT thread.
- UI/daemon → RT: lock-free only (`atomic`, `rtrb`); a chain change is an atomic swap, the old chain is
  freed outside the RT thread.
- Parameters are smoothed (no clicks), denormals flushed to zero, NaN/Inf sanitized at the chain output.
- A module reports its latency (`latency_samples`); a disabled module costs 0 CPU and 0 latency.
- A new DSP module or an RT path change → run its tests under `assert_no_alloc`.

## 5. Frontend (Tauri v2 + Svelte 5)

- Svelte 5 (runes), strict TypeScript, Vite, pnpm.
- Styles — **only** tokens from `docs/design/design-system.md` (CSS variables). No arbitrary hex values,
  radii, shadows or fonts. Dark theme only. `border-radius: 0`.
- Fonts are bundled locally, no network requests (CDN, Google Fonts, analytics).
- UI strings go through the i18n layer (only `en` for now), not hardcoded in markup.
- The UI knows nothing about audio directly — only IPC to the daemon. Protocol types come from
  `macafon-ipc` (TS type generation), never duplicated by hand.
- **Daemon mock mode** is mandatory: the UI must run in a regular browser
  (`pnpm -C apps/macafon-ui dev` with a mock backend serving plausible devices, apps and live meters).
  It is needed for development and screenshots.
- Accessibility from PRD §9.8 (keyboard, ARIA for sliders) — right away, not "later".

## 6. Screenshots at the end of a round

**If the UI changed in the round**, attach screenshots of the changed screens/states to the final
report. If the UI did not change, no screenshots are needed.

- Run the UI in mock mode, window 1180×760 (the default size from the PRD).
- Capture only what changed: the full screen plus a close-up of the component if the edit is small;
  important states (hover/drag/disabled/error) as separate frames if they changed.
- Save to `.artifacts/screenshots/<YYYY-MM-DD>-<topic>-<screen>.png` (the folder is in `.gitignore`,
  never committed) and embed them in the report by absolute path.
- If a screenshot could not be taken, say so in the report and why.

## 7. End-of-round report

Short:
1. What was done (with PRD requirement IDs).
2. How it was checked — which commands were run and their results. Not run — say so.
3. Screenshots (only if the UI changed).
4. What was not done / known issues / questions for the owner.

## 8. Forbidden without the owner's explicit permission

- Cloud CI beyond the authorized manual Windows installer workflow.
- Telemetry, analytics, network requests (except the opt-in update check from the PRD and
  user-initiated dictation model downloads from pinned Hugging Face URLs, PRD §8.4.6 — owner
  authorization, 2026-10-19).
- Root/administrator actions at runtime beyond what the PRD describes (none on Windows, except the
  WIN-MIC-010 option).
- Changing the user's system audio settings without restoring the original state.
- Bundling VB-Cable or other third-party drivers into the installer.
- Changing the stack (Tauri, Svelte, PipeWire API) or accepted ADRs.

## 9. Commands

```sh
cargo fmt
cargo clippy -p <crate> -- -D warnings
cargo test -p <crate> [filter]
cargo build --release -p macafond -p macafon-cli
target/release/macafond run --input '<node.name>'   # daemon (PipeWire + control socket)
target/release/macafon ctl status                   # control the daemon
python3 tools/tests/pipewire-smoke.py             # private PipeWire: latency, mute, restarts
python3 tools/tests/wireplumber-routing.py        # private WirePlumber: routing
python3 tools/tests/mixer-smoke.py                # private WirePlumber: channels, apps, outputs
python3 tools/tests/hotkeys-smoke.py              # fake GlobalShortcuts portal: hotkeys, OSD
python3 tools/tests/control-smoke.py              # live control, config, protocol
python3 tools/tests/tray-smoke.py                 # tray over a private D-Bus session
python tools/tests/windows-uninstall-hook.py      # Windows: uninstaller stops on a failed restore
pnpm -C apps/macafon-ui dev              # UI in a browser with the mock daemon
pnpm -C apps/macafon-ui tauri dev        # UI in Tauri with the real daemon
pnpm -C apps/macafon-ui check            # svelte-check / TypeScript
pnpm -C apps/macafon-ui gen:ipc          # regenerate TS types from macafon-ipc/macafon-dsp
python tools/package-windows.py          # Windows: per-user NSIS installer (pnpm via corepack)
python tools/bump-version.py patch       # version bump before a push to main (§2)
```
