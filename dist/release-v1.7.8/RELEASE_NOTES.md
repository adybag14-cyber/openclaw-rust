# openclaw-agent-rs v1.7.8 (edge)

## Highlights

- Expanded `tools.catalog` parity payload shape with typed params (`agentId`, `includePlugins`) and upstream-aligned fields (`agentId`, `profiles`, `groups`).
- Added strict `tools.catalog` unknown-agent validation and retained deny-unknown request-shape enforcement.
- Expanded CLI parity surface:
  - Top-level `status` and `health` commands.
  - Top-level `tools catalog` command with `--agent-id` and `--include-plugins`.
  - Direct RPC invocation via `gateway call --method --params`.
- Added parser regression coverage for new CLI surfaces.
- Refreshed parity artifacts and scoreboard:
  - Rust methods: `132`
  - Coverage: `100%` vs upstream base + handlers

## Validation

- Windows GNU:
  - `cargo +1.83.0-x86_64-pc-windows-gnu fmt --all -- --check`
  - `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
  - `cargo +1.83.0-x86_64-pc-windows-gnu test`
  - `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
  - `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state"`
  - `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets --features sqlite-state -- -D warnings"`
  - `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu build --release --features sqlite-state"`
- Ubuntu 20.04 (WSL):
  - `wsl -d Ubuntu-20.04 -- bash -lc 'source $HOME/.cargo/env && cd /mnt/c/Users/Ady/Documents/openclaw-rust && CARGO_TARGET_DIR=target-linux-ubuntu20 cargo +1.83.0 build --release'`
- Parity + Docker:
  - `./scripts/parity/method-surface-diff.ps1 -Surface both -UpstreamRepoPath ..\\openclaw`
  - `./scripts/parity/build-scoreboard.ps1 -IncludeGeneratedAt`
  - `./scripts/parity/run-cp0-gate.ps1 -UpstreamRepoPath ..\\openclaw`
  - `docker buildx prune -af`
  - `./scripts/run-docker-parity-smoke.ps1`
