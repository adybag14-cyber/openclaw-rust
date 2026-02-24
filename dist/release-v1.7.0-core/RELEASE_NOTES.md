# openclaw-rust-core v1.7.0

## Highlights

- Version bump to `1.7.0` with runtime hardening and edge-control-path upgrades merged into the Rust gateway.
- Added executable enclave attestation bridge plumbing (`OPENCLAW_RS_ENCLAVE_ATTEST_BIN`) with cached proof records exposed through `edge.enclave.status`.
- Added executable LoRA trainer orchestration path for `edge.finetune.run` (`dryRun=false`) with timeout and log-tail capture.
- Added mesh runtime probe telemetry on `edge.mesh.status` (`mesh.ping` invoke wait, timeout/failure summaries).

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
- Docker parity smoke attempted: `./scripts/run-docker-parity-smoke.ps1` currently fails in this workstation due Docker Desktop memory limit (`cannot allocate memory`).
