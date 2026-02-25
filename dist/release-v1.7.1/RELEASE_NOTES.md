# openclaw-agent-rs v1.7.1

## Highlights

- Added Telegram `/model` command support for provider/model listing and live session model overrides.
- Added Telegram `/set api key <provider> <key>` command for runtime provider credential patching.
- Expanded provider alias normalization for models.dev naming variants mapped to existing runtime providers.

## Validation

- `cargo +1.83.0-x86_64-pc-windows-gnu fmt --all -- --check`
- `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-gnu test`
- `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state"`
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu build --release --features sqlite-state"`
- `./scripts/parity/run-cp0-gate.ps1`
- `./scripts/parity/run-cp6-gate.ps1`
- Ubuntu 20.04 (WSL): `cargo +1.83.0 check`, `cargo +1.83.0 test --no-run`, `cargo +1.83.0 build --release`
- Docker parity smoke attempted via `./scripts/run-docker-parity-smoke.ps1` (daemon unavailable on this workstation during release build).
