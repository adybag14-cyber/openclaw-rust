## Highlights
- Added native Rust Telegram bridge runtime execution in standalone mode, including Bot API polling, policy gating, RPC agent execution, status events, and durable update offsets.
- Fixed standalone runtime bridge task lifecycle handling so shutdown is clean and deterministic.
- Added `PROJECT_OVERVIEW.md` as a full technical project/system overview for architecture, security, provider runtime, persistence, performance, and release operations.

## Validation
- Windows GNU toolchain:
  - `cargo +1.83.0-x86_64-pc-windows-gnu fmt --all -- --check`
  - `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
  - `cargo +1.83.0-x86_64-pc-windows-gnu test`
  - `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- Ubuntu 20.04 (WSL2):
  - `CARGO_TARGET_DIR=target-linux cargo +1.83.0 build --release`
