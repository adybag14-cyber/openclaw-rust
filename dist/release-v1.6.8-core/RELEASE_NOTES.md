# openclaw-rust-core v1.6.8

## Highlights

- Revalidated the full core baseline with no regressions.
- Version bump aligned with the new edge capability release (`v1.6.8`).
- Core remains conservative by default while keeping compatibility with all new edge surfaces.

## Validation

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- `cargo test --features sqlite-state`
- `./scripts/parity/run-cp0-gate.ps1`
- `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-gnu test`
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state"`
- `cargo +1.83.0-x86_64-pc-windows-msvc build --release`
- `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- Ubuntu 20.04 (WSL): `cargo +1.83.0 check`, `cargo +1.83.0 test --no-run`, `cargo +1.83.0 build --release`
- Runtime QA: `doctor --non-interactive --json`, `security audit --deep --json`
