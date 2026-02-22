# OpenClaw Agent Rust v1.6.2

## Highlights
- Added real `wasmtime` runtime execution for the `wasm` tool path with capability/fuel/memory policy enforcement.
- Added dynamic WIT tool loading and schema generation (`wasm` actions: `registry`, `schema`).
- Added credential policy `secret_names` support and stronger bidirectional request/response leak detection + redaction.
- Expanded SafetyLayer integration for input/output scanning, control-char cleanup, truncation, and policy escalation.
- Added new `[security.wasm]` config section and synced runtime-mode controls into tool runtime policy.
- Added doctor wasm checks (`security.wasm_runtime_mode`, `security.wasm_wit_root`, `security.wasm_module_root`, `wasmtime.binary`).

## Validation
- Windows (MSVC):
  - `cargo fmt --all`
  - `cargo check`
  - `cargo test`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo build --release`
  - `cargo run -- doctor --non-interactive --json`
- WSL (`Ubuntu 22.04.5 LTS`):
  - `CARGO_TARGET_DIR=target-linux cargo +1.83.0 check`
  - `CARGO_TARGET_DIR=target-linux cargo +1.83.0 test --no-run`
  - `CARGO_TARGET_DIR=target-linux cargo +1.83.0 build --release`
- Docker (`ubuntu:20.04.6 LTS`):
  - `CARGO_TARGET_DIR=target-docker cargo +1.83.0 check`
  - `CARGO_TARGET_DIR=target-docker cargo +1.83.0 build --release`
  - `./target-docker/release/openclaw-agent-rs doctor --non-interactive --json`

## Notes
- On this workstation, `docker ... cargo test --no-run` hit container memory limits (`SIGKILL`).
- Release binaries were produced from validated Windows and Ubuntu 20.04-targeted Docker builds.
