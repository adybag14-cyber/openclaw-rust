# OpenClaw Rust Core + Edge Implementation Plan

Source inputs:
- `C:\Users\adyba\Downloads\table (3).csv` (long-horizon feature set)
- `C:\Users\adyba\Downloads\table (4).csv` (current practical feature set and status)

Date:
- 2026-02-24

## Goal

Ship two release tracks from one Rust codebase:

- `openclaw-rust-core`: minimal, secure, low-RAM baseline.
- `openclaw-rust-edge`: advanced capability track with optional heavier features.

## Constraints

- Keep single-binary principle for runtime executable.
- Keep deterministic release validation for Windows + Ubuntu 20.04 targets.
- Avoid regressions in gateway, tool runtime, and security surfaces.

## Current State Summary

Already present (from `table (4)` and repo state):

- [x] Persistent Vector + Graph Memory (`v1.6.4`)
- [x] WASM Sandbox + SafetyLayer
- [x] Security Audit CLI (`security audit --deep --fix`)
- [x] Keyless + provider matrix runtime
- [x] Doctor + CLI parity checks
- [x] Clean single binary baseline

Partially present / next upgrade targets:

- [x] Native Offline Voice (lazy local provider path shipped via `kittentts`)
- [x] Self-Healing Orchestrator (light runtime fallback retries + telemetry shipped)

## Track Definition

## Core Track (`openclaw-rust-core`)

Primary design:

- [ ] Keep memory + security + doctor surfaces enabled by default.
- [ ] Keep offline voice optional and lazy (off until explicitly used).
- [ ] Keep orchestrator retry conservative (bounded attempts).
- [ ] Prefer low-RAM defaults for long-running deployments.

Target profile characteristics:

- [ ] deterministic startup
- [ ] bounded memory growth
- [ ] no heavy optional workers by default

## Edge Track (`openclaw-rust-edge`)

Primary design:

- [ ] Include Core baseline plus advanced optional capabilities.
- [ ] Enable richer voice and recovery behavior (still policy-gated).
- [ ] Favor resilience and feature depth over strict minimal RAM.

Target profile characteristics:

- [ ] broader model/runtime adaptation
- [ ] richer voice pipeline options
- [ ] expanded runtime telemetry

## Feature Roadmap Mapping (table 3 -> practical phases)

Immediate (next build window):

- [x] Native Offline Voice pipeline (basic lazy-loaded path)
- [x] Self-Healing Orchestrator (light runtime recovery + fallback)

Near-term (next releases):

- [ ] GPU/NPU acceleration hooks
- [ ] Full multimodal pipeline
- [ ] Agent swarm expansion

Long-term (research/high-complexity):

- [ ] Hardware enclaves + zero-knowledge runtime
- [ ] Homomorphic encryption mode
- [ ] Decentralized P2P agent mesh
- [ ] On-device fine-tuning / self-evolution

## Implementation Plan (detailed checklist)

## A) Runtime and Feature Controls

- [ ] Add explicit core/edge profile guidance in docs and release notes.
- [ ] Add configuration toggles for profile-sensitive behavior.
- [ ] Ensure profile toggles are additive and backwards-compatible.

## B) Feature 1: Native Offline Voice (lazy)

- [x] Add offline provider surface (`kittentts`) to TTS provider set.
- [x] Add offline provider metadata to `tts.status` and `tts.providers`.
- [x] Keep lazy behavior: offline binary only used if configured.
- [ ] Add optional transcription path (tiny-whisper style) in follow-up.
- [ ] Add profile default differences (core disabled-by-default, edge recommended).

Acceptance checks:

- [x] `tts.setProvider kittentts` accepted.
- [x] `tts.convert` returns audio payload with provider tracking.
- [x] Missing local binary gracefully falls back without crashing.

## C) Feature 2: Self-Healing Orchestrator (light)

- [x] Add bounded runtime self-heal retry attempts on agent turn failure.
- [x] Retry against fallback providers when runtime call fails.
- [x] Emit structured runtime self-healing telemetry (`runtime.selfHealing`).
- [ ] Add policy knobs for max attempts and backoff tuning (follow-up).

Acceptance checks:

- [x] Primary provider failure can recover via fallback provider.
- [x] Recovery attempts are visible in response payload.
- [x] Failed retries terminate cleanly with final deterministic error.

## D) Validation Matrix

Windows:

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test`
- [x] `cargo +1.83.0-x86_64-pc-windows-msvc build --release`

Ubuntu 20.04 (WSL):

- [x] Ensure rustup/cargo installed in `Ubuntu-20.04`.
- [x] `cargo +1.83.0 check`
- [x] `cargo +1.83.0 test --no-run`
- [x] `cargo +1.83.0 build --release`

Parity/security:

- [x] Run relevant parity gate scripts for touched surfaces.
- [x] Run `doctor --non-interactive --json`.
- [x] Run `security audit --deep --json`.
- [x] Note parity gate environment gap: CP0 wrapper requires GNU linker setup with `-lgcc_eh`; MSVC + Ubuntu matrix and corpus tests passed.

## E) Packaging and Dist Output

- [x] Produce Windows binary artifact for next build.
- [x] Produce Ubuntu 20.04 binary artifact for next build.
- [x] Create core/edge packaging directories with notes and checksums.
- [x] Document artifact provenance (toolchain + OS target).

## F) GitHub Release Workflow

- [x] Open planning issue with checklist tracking this plan.
- [ ] Link implementation PR(s) to the issue.
- [x] Attach build artifacts + notes after validation pass.

## Risks and Mitigations

Risk: self-heal retries create long tail latency.  
Mitigation:
- [ ] keep low max attempts
- [ ] keep explicit timeout and clear error surface

Risk: offline voice integration causes brittle local dependency behavior.  
Mitigation:
- [ ] lazy-load only
- [ ] graceful fallback to simulated/edge mode
- [ ] explicit status visibility of binary availability

Risk: profile drift between core and edge.  
Mitigation:
- [ ] define profile contract in docs
- [ ] include profile checks in doctor/security outputs

## Definition of Done for This Build Start

- [x] Plan drafted from table (3) and table (4).
- [x] GitHub issue created with full checklist.
- [x] Two significant feature implementations started in codebase.
- [x] Windows release binary compiled and staged.
- [x] Ubuntu 20.04 release binary compiled and staged.
