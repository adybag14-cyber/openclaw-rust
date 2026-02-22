# Release v1.6.4

## Highlights
- Added persistent `zvec`-style vector memory backend in Rust with disk snapshots and cosine top-k recall.
- Added persistent `graphlite`-style graph memory backend in Rust with session/concept node and edge accumulation.
- Integrated memory ingestion and recall directly into `agent` runtime execution (`agent.user`/`agent.assistant` turn memory).
- Added memory runtime telemetry in `gateway status` and `health` payloads.
- Added runtime config parsing for `memory.*` controls (`enabled`, store paths, max entries, recall top-k, min score).

## Validation
- `cargo fmt --all`
- `cargo check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-msvc check`
- `cargo +1.83.0-x86_64-pc-windows-msvc test --no-run`
- `cargo +1.83.0-x86_64-pc-windows-msvc build --release`
- `wsl -d Ubuntu bash -lc '... CARGO_TARGET_DIR=target-linux cargo +1.83.0 check'`
- `wsl -d Ubuntu bash -lc '... CARGO_TARGET_DIR=target-linux cargo +1.83.0 test --no-run'`
- `wsl -d Ubuntu bash -lc '... CARGO_TARGET_DIR=target-linux cargo +1.83.0 build --release'`
- `docker run --rm -v "C:/Users/adyba/openclaw-rust:/workspace" -w /workspace ubuntu:20.04 bash -lc '... CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=target-docker cargo +1.83.0 build --release -q && ./target-docker/release/openclaw-agent-rs doctor --non-interactive --json'`

## Notes
- Docker Ubuntu 20.04 `cargo +1.83.0 test --no-run -q` exceeded the command timeout on this workstation.
