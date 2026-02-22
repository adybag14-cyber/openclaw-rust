# OpenClaw Agent Rust v1.6.3

## Highlights
- Added native `security audit` CLI parity with `--deep`, `--fix`, and `--json` modes.
- Added structured security findings report output (summary + findings + optional deep gateway probe block).
- Added deterministic safe-fix actions for common hardening issues and filesystem permission tightening where supported.
- Added CP7 CLI parity fixture coverage for the `security audit` command.
- Bumped runtime/tooling contract tags to `v1.6.3`.

## Validation
- Windows (MSVC):
  - `cargo check`
  - `cargo test`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo build --release`
  - `cargo run -- doctor --non-interactive --json`
  - `cargo run -- security audit --deep --json`
  - `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\parity\run-cp7-gate.ps1 -Toolchain 1.83.0-x86_64-pc-windows-msvc`
- WSL (`Ubuntu`):
  - `CARGO_TARGET_DIR=target-linux cargo +1.83.0 check`
  - `CARGO_TARGET_DIR=target-linux cargo +1.83.0 test --no-run`
  - `CARGO_TARGET_DIR=target-linux cargo +1.83.0 build --release`
- Docker (`ubuntu:20.04.6 LTS`):
  - `CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=target-docker cargo +1.83.0 build --release -j 1`
  - `./target-docker/release/openclaw-agent-rs doctor --non-interactive --json`

## Notes
- Initial unconstrained docker release build hit memory pressure (`SIGKILL`); constrained `CARGO_BUILD_JOBS=1` build succeeded.
- `docker ... cargo test --no-run` remained heavy on this workstation and exceeded a 30-minute timeout during validation attempts.
