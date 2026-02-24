# openclaw-rust-core v1.6.9

## Highlights

- Added first-class Qwen 3.5 guest bridge routing with multi-model fallback (`qwen3.5-397b-a17b`, `qwen3.5-plus`, `qwen3.5-flash`) and OpenAI-shape response translation.
- Added Inception/Mercury provider normalization and defaults, including `mercury-2` catalog support.
- Kept core runtime stable while aligning provider/bridge and model registry behavior with edge expansions.

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
