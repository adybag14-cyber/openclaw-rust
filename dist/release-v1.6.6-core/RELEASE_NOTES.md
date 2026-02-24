# openclaw-rust v1.6.6-core

## Scope
- Core track release artifact for v1.6.6.
- Adds profile-aware runtime defaults and configurable agent self-healing policy knobs.

## Highlights
- runtime.profile support (`core` / `edge`) with conservative core defaults.
- self-healing policy config:
  - runtime.selfHealing.enabled
  - runtime.selfHealing.maxAttempts
  - runtime.selfHealing.backoffMs
- tts.status now exposes runtime profile and offline recommendation metadata.

## Artifacts
- openclaw-rust-core-windows-x86_64.exe
- openclaw-rust-core-v1.6.6-windows-x86_64.zip
- openclaw-rust-core-ubuntu20.04-x86_64
- openclaw-rust-core-v1.6.6-ubuntu20.04-x86_64.tar.gz

## Validation
- cargo fmt --all -- --check
- cargo clippy --all-targets -- -D warnings
- cargo test
- cargo test --features sqlite-state
- ./scripts/parity/run-cp0-gate.ps1
- cargo +1.83.0-x86_64-pc-windows-msvc build --release
- Ubuntu-20.04 WSL: cargo +1.83.0 check/test --no-run/build --release
