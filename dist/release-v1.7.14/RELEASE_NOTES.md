# OpenClaw Rust v1.7.14

Release date: 2026-02-28

## Highlights
- Added full Rust parity support for upstream `secrets.reload`.
- Implemented gateway handler/runtime sync path for secrets snapshot reloads.
- Restored method-surface parity to 100% against latest upstream release surface.

## Validation Matrix (passed)
- `cargo +1.83.0-x86_64-pc-windows-gnu fmt --all -- --check`
- `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-gnu test` (`406 passed, 1 ignored`)
- `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state"` (`410 passed, 1 ignored`)
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets --features sqlite-state -- -D warnings"`
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu build --release --features sqlite-state"`
- `./scripts/parity/method-surface-diff.ps1 -Surface both -UpstreamRepoPath ..\openclaw`
- `./scripts/parity/build-scoreboard.ps1 -IncludeGeneratedAt`
- `./scripts/parity/run-cp0-gate.ps1 -UpstreamRepoPath ..\openclaw`
- `./scripts/run-docker-parity-smoke.ps1`
- `wsl -d Ubuntu-20.04 -- bash -lc 'source $HOME/.cargo/env && cd /mnt/c/Users/Ady/Documents/openclaw-rust && CARGO_TARGET_DIR=target-linux-ubuntu20 cargo +1.83.0 build --release'`

## Assets
- `openclaw-agent-rs-windows-x86_64.exe`
- `openclaw-agent-rs-linux-ubuntu20.04-x86_64`
- `openclaw-agent-rs-v1.7.14-windows-x86_64.zip`
- `openclaw-agent-rs-v1.7.14-linux-ubuntu20.04-x86_64.tar.gz`
