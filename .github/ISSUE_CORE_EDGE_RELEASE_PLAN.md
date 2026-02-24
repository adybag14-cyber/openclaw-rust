# Plan: OpenClaw Rust Core + Edge (table 3 / table 4)

Tracking issue for dual release-track execution:

- `openclaw-rust-core`
- `openclaw-rust-edge`

Reference plan:
- `CORE_EDGE_RELEASE_PLAN_TABLE3_TABLE4.md`

## Objectives

- [ ] Maintain single codebase + single-binary runtime principle.
- [ ] Deliver validated binaries for Windows and Ubuntu 20.04.
- [ ] Stage advanced features safely (no regression in security/parity paths).

## Immediate Implementation Targets

### Feature 1: Native Offline Voice (lazy)
- [x] Add `kittentts` offline provider surface.
- [x] Add offline metadata to `tts.status`.
- [x] Add offline provider entry to `tts.providers`.
- [x] Ensure graceful fallback when local binary is absent.
- [ ] Add optional offline transcription path (follow-up).

### Feature 2: Self-Healing Orchestrator (light)
- [x] Add bounded runtime self-heal retry path for `agent` execution failures.
- [x] Retry across fallback provider chain.
- [x] Emit structured self-healing telemetry in runtime payload.
- [ ] Add policy knobs for retry/backoff tuning (follow-up).

## Core Track Checklist

- [ ] Keep memory/security/doctor defaults stable.
- [ ] Keep optional heavy subsystems lazy/off by default.
- [ ] Keep bounded retry and low-RAM behavior as baseline.
- [ ] Validate no CLI/gateway parity regressions.

## Edge Track Checklist

- [ ] Keep core baseline plus richer optional capabilities.
- [ ] Keep offline voice and advanced resilience surfaces enabled/recommended by profile guidance.
- [ ] Validate expanded behavior does not break core defaults.

## Build and Validation Matrix

### Windows
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`
- [ ] `cargo +1.83.0-x86_64-pc-windows-msvc build --release`

### Ubuntu 20.04 (WSL)
- [ ] Install rust toolset in `Ubuntu-20.04` if missing.
- [ ] `cargo +1.83.0 check`
- [ ] `cargo +1.83.0 test --no-run`
- [ ] `cargo +1.83.0 build --release`

### Runtime QA
- [ ] `doctor --non-interactive --json`
- [ ] `security audit --deep --json`

## Packaging

- [ ] Stage Windows artifact in `dist/` for next build.
- [ ] Stage Ubuntu 20.04 artifact in `dist/` for next build.
- [ ] Add checksums and release-note draft for core/edge naming.

## Risk Controls

- [ ] Keep retries bounded and observable.
- [ ] Keep offline dependencies optional and discoverable through status endpoints.
- [ ] Keep explicit rollback path for feature regressions.

