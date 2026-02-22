# OpenClaw Agent Rust v1.5.2

## Highlights
- Added WASM runtime surface with capability-gated sandbox policy support.
- Added WIT contract scaffold at wit/tool.wit.
- Added credential injection allowlist and bidirectional leak detection/redaction.
- Added layered safety controls for output sanitization/truncation.
- Added routines orchestrator tool surface.
- Updated MinGW helper script to auto-detect valid workstation toolchains.

## Validation
- cargo +1.83.0-x86_64-pc-windows-gnu fmt --all -- --check
- cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings
- cargo +1.83.0-x86_64-pc-windows-gnu test
- cargo +1.83.0-x86_64-pc-windows-gnu build --release
- cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets --features sqlite-state -- -D warnings
- cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state
- cargo +1.83.0-x86_64-pc-windows-gnu build --release --features sqlite-state
