# openclaw-rust-edge v1.6.9

## Highlights

This release ships the requested edge expansion and table (6) completion items:

- Added Qwen 3.5 bridge parity path (guest-web bridge mode) with resilient model fallback and SSE/content normalization.
- Added Mercury 2 provider support through Inception provider defaults and alias normalization.
- Added `edge.finetune.status` and `edge.finetune.run` RPCs for the on-device fine-tune/self-evolution feature line, with bounded policy controls for epochs/rank/samples and trainer wiring hooks.

With this release, the final remaining edge feature from `table (6).csv` is represented in the Rust runtime alongside prior enclave/mesh/homomorphic surfaces.

## Validation

- `cargo +1.83.0-x86_64-pc-windows-gnu fmt --all -- --check`
- `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-gnu test`
- `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state"`
- `./scripts/parity/run-cp0-gate.ps1`
- `cargo +1.83.0-x86_64-pc-windows-msvc build --release`
- Ubuntu 20.04 (WSL): `cargo +1.83.0 check`, `cargo +1.83.0 test --no-run`, `cargo +1.83.0 build --release`
- Runtime QA: `cargo run -- doctor --non-interactive --json`, `cargo run -- security audit --deep --json`
- Docker parity image smoke attempted; runtime daemon recovered, build then failed under current Docker Desktop memory cap (`cannot allocate memory`).
