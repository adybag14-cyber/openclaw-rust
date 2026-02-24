# openclaw-rust-edge v1.7.0

## Highlights

This release upgrades the table (7) feature line from surface-level status methods into executable runtime flows in Rust:

- `Hardware Enclaves + Zero-Knowledge Mode`
  - `edge.enclave.prove` now supports configured attestation-binary execution (`OPENCLAW_RS_ENCLAVE_ATTEST_BIN` + args) and emits persisted proof records with quote/measurement metadata.
  - `edge.enclave.status` now reports attestation configuration and last-proof state.
- `On-device Fine-Tuning / Self-Evolution`
  - `edge.finetune.run` now executes trainer processes when `dryRun=false` (requires `OPENCLAW_RS_LORA_TRAINER_BIN`), with bounded timeout, exit status, and log-tail capture.
  - `edge.finetune.status` now includes persisted job history and job statistics.
- `Decentralized P2P Agent Mesh`
  - `edge.mesh.status` now supports runtime mesh probing via `mesh.ping` invoke-wait flows and returns health telemetry (`successCount`, `timeoutCount`, failed peers, probe details).

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
