# openclaw-agent-rs v1.7.6 (edge)

## Highlights

- Hardened Rust-native website bridges for `zai`, `qwen-portal`, and `inception` with better fallback behavior across stale-key, keyless, and loopback-bridge scenarios.
- Added effective status handling (`x-actual-status-code`) so wrapped upstream responses are interpreted correctly during bridge retries.
- Updated Inception/Mercury guest bridge flow to attempt direct completions first (`/api/v1/chat/completions`, `/api/chat/completions`), with legacy chat-create flow retained as fallback.
- Expanded Qwen guest bridge support with v2 request shapes and robust chat ID extraction while keeping v1 auth fallback compatibility.
- Improved provider runtime diagnostics so direct API failures now include website-bridge fallback error context.
- Added targeted regression tests for stale-key fallback paths, loopback endpoint detection, and bridge model routing.

## Validation

- Windows GNU:
  - `cargo +1.83.0-x86_64-pc-windows-gnu fmt --all -- --check`
  - `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
  - `cargo +1.83.0-x86_64-pc-windows-gnu test`
  - `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
  - `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state"`
- Ubuntu 20.04 (WSL):
  - `cargo +1.83.0 fmt --all -- --check`
  - `cargo +1.83.0 clippy --all-targets -- -D warnings`
  - `cargo +1.83.0 test`
  - `cargo +1.83.0 build --release`
- Parity and container:
  - `./scripts/parity/run-cp6-gate.ps1`
  - `./scripts/parity/run-cp0-gate.ps1`
  - `./scripts/run-docker-parity-smoke.ps1`
