# openclaw-rust-edge v1.6.8

## Highlights

This release completes the remaining planned features from `table (5).csv`:

- `edge.enclave.status` + `edge.enclave.prove`
  - Hardware enclave runtime signal surface (SGX/TPM/SEV) and deterministic zero-knowledge-style commitment proof contract.
- `edge.mesh.status`
  - Decentralized P2P mesh topology/reporting across paired nodes/devices with secure-route metadata.
- `edge.homomorphic.compute`
  - Homomorphic-mode compute surface for encrypted-domain operations (`sum`, `count`, `mean` with reveal gate).

With this, all feature lines listed in table (5) are now represented in the Rust runtime.

## Validation

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- `cargo test --features sqlite-state`
- `./scripts/parity/run-cp0-gate.ps1`
- `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-gnu test`
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state"`
- `cargo +1.83.0-x86_64-pc-windows-msvc build --release`
- `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- Ubuntu 20.04 (WSL): `cargo +1.83.0 check`, `cargo +1.83.0 test --no-run`, `cargo +1.83.0 build --release`
- Runtime QA: `doctor --non-interactive --json`, `security audit --deep --json`
