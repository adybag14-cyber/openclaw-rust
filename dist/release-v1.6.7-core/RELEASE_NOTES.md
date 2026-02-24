# openclaw-rust-core v1.6.7

## Highlights

- Core baseline revalidated as complete against table (4): persistent memory, wasm safetylayer, security audit CLI, provider matrix, doctor/CLI parity, single-binary runtime, offline voice + self-healing baseline.
- Runtime profile contract remains stable (`runtime.profile=core`) with bounded retries and conservative defaults.
- Includes shared runtime improvements shipped in this build without enabling edge-only behavior by default.

## Validation

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- `cargo test --features sqlite-state`
- `./scripts/parity/run-cp0-gate.ps1`
- `cargo +1.83.0-x86_64-pc-windows-msvc build --release`
- `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- Ubuntu 20.04 (WSL): `cargo +1.83.0 check`, `cargo +1.83.0 test --no-run`, `cargo +1.83.0 build --release`
- Runtime QA: `doctor --non-interactive --json`, `security audit --deep --json`
