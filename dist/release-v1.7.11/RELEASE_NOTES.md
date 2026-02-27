# OpenClaw Rust v1.7.11

## Highlights
- Hardened ChatGPT OAuth/browser-auth runtime for cross-platform and restart-safe operation.
- OAuth credential store now defaults to `.openclaw-rs/oauth/sessions.json` (disk-backed persistence).
- Added runtime/env overrides for OAuth store path, browser auth command/args/profile, and ChatGPT bridge base URL candidates.
- Added Linux-safe default browser auth launch path (`xvfb-run -a node scripts/chatgpt-browser-auth.mjs --engine puppeteer`) with improved diagnostics for missing browser/display dependencies.
- Added `gateway call --live-service` to execute RPC calls against the live gateway websocket service (challenge + auth + request/response flow).

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
