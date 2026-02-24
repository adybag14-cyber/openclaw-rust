# openclaw-rust-edge v1.6.7

## Highlights

- Adds next 6 significant edge capabilities from table (3):
  - `edge.voice.transcribe` (offline transcription pipeline with tinywhisper + deterministic fallback)
  - `edge.router.plan` (smart model router planning with objective-aware provider chains)
  - `edge.acceleration.status` (GPU/NPU acceleration detection hooks)
  - `edge.wasm.marketplace.list` (WASM marketplace inventory + builder scaffolding metadata)
  - `edge.swarm.plan` (autonomous swarm plan generation)
  - `edge.multimodal.inspect` (vision/screen/text multimodal inspection contract)
- Keeps core baseline intact while expanding edge-only capability surfaces.

## Validation

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- `cargo test --features sqlite-state`
- `./scripts/parity/run-cp0-gate.ps1`
- `cargo +1.83.0-x86_64-pc-windows-msvc build --release`
- `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- Ubuntu 20.04 (WSL): `cargo +1.83.0 check`, `cargo +1.83.0 test --no-run`, `cargo +1.83.0 build --release`
- Runtime QA: `doctor --non-interactive --json`, `security audit --deep --json`
