# openclaw-agent-rs v1.7.2

## Highlights

- Implemented the remaining table (8) features in Rust (excluding autonomous self-forking):
  - `edge.identity.trust.status`
  - `edge.personality.profile`
  - `edge.handoff.plan`
  - `edge.marketplace.revenue.preview`
  - `edge.finetune.cluster.plan`
  - `edge.alignment.evaluate`
  - `edge.quantum.status`
  - `edge.collaboration.plan`
- Added Telegram OAuth runtime control commands:
  - `/auth providers`
  - `/auth start <provider> [account] [--force]`
  - `/auth wait <provider> [session_id] [account]`
  - `/auth wait session <session_id> [account]`
- Updated docs and parity metadata for the expanded edge capability set.

## Validation

- `cargo +1.83.0-x86_64-pc-windows-gnu fmt --all -- --check`
- `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-gnu test`
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state"`
- `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- Ubuntu 20.04 (WSL): `cargo +1.83.0 build --release`
- Docker parity smoke: `./scripts/run-docker-parity-smoke.ps1`
- Ubuntu 20.04 memory profile under active RPC traffic: `MAX_RSS_KB=15744` (`MAX_RSS_MB=15.38`)
