# OpenClaw Rust v1.7.10

## Highlights
- Added `scripts/chatgpt-browser-bridge.mjs` to expose a local OpenAI-compatible `/v1/chat/completions` + `/health` bridge backed by an authenticated ChatGPT browser session.
- Added model alias support for browser-session slugs (`gpt-5.2-pro`, `gpt-5.2-thinking`, `gpt-5.2-instant`, `gpt-5.2-auto`, `gpt-5.2`, `gpt-5.1`, `gpt-5-mini`) and ChatGPT bridge fallback normalization.
- Updated OpenAI OAuth runtime binding to include loopback bridge candidates (`http://127.0.0.1:43010/v1`, `http://127.0.0.1:43010`) plus ChatGPT origins.
- Updated docs/examples for browser-session setup (`README.md`, `openclaw-rs.example.toml`).
- Hardened standalone gateway control HTTP integration coverage by adding retry handling to the GET helper used in the control-plane test suite.

## Validation
- `cargo +1.83.0-x86_64-pc-windows-gnu fmt --all -- --check`
- `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-gnu test`
- `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state"`
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets --features sqlite-state -- -D warnings"`
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu build --release --features sqlite-state"`
- `./scripts/parity/method-surface-diff.ps1 -Surface both -UpstreamRepoPath ..\openclaw`
- `./scripts/parity/build-scoreboard.ps1 -IncludeGeneratedAt`
- `./scripts/parity/run-cp0-gate.ps1 -UpstreamRepoPath ..\openclaw`
- `./scripts/run-docker-parity-smoke.ps1`
- `wsl -d Ubuntu-20.04 -- bash -lc 'source $HOME/.cargo/env && cd /mnt/c/Users/Ady/Documents/openclaw-rust && CARGO_TARGET_DIR=target-linux-ubuntu20 cargo +1.83.0 build --release'`
- `node --check scripts/chatgpt-browser-bridge.mjs`
