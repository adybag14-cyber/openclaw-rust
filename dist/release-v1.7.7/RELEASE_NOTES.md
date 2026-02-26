# openclaw-agent-rs v1.7.7 (edge)

## Highlights

- Completed missing gateway RPC method-surface parity by adding `doctor.memory.status` and `tools.catalog` to Rust dispatcher/runtime support.
- Added strict request-shape validation and deterministic response payload handling for both new methods.
- Refreshed parity artifacts and scoreboard to reflect full method-surface parity:
  - Rust methods: `132`
  - Coverage: `100%` vs upstream base + handlers
- Preserved full Windows GNU validation matrix pass, including `sqlite-state` build/test/clippy variants.
- Re-ran parity and container gates for release confidence:
  - `./scripts/parity/run-cp0-gate.ps1 -UpstreamRepoPath ..\\openclaw`
  - `./scripts/run-docker-parity-smoke.ps1`

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
  - `cargo +1.83.0 build --release`
- Parity + Docker:
  - `./scripts/parity/method-surface-diff.ps1 -Surface both -UpstreamRepoPath ..\\openclaw`
  - `./scripts/parity/build-scoreboard.ps1 -IncludeGeneratedAt`
  - `./scripts/parity/run-cp0-gate.ps1 -UpstreamRepoPath ..\\openclaw`
  - `./scripts/run-docker-parity-smoke.ps1`