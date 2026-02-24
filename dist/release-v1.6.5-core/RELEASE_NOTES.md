# openclaw-rust v1.6.5-core

## Scope
- Core track release artifact for v1.6.5.
- Includes self-healing runtime retry telemetry and offline kittentts provider surface.

## Artifacts
- openclaw-rust-core-windows-x86_64.exe
- openclaw-rust-core-v1.6.5-windows-x86_64.zip
- openclaw-rust-core-ubuntu20.04-x86_64
- openclaw-rust-core-v1.6.5-ubuntu20.04-x86_64.tar.gz

## Validation
- cargo fmt --all -- --check
- cargo clippy --all-targets -- -D warnings
- cargo test
- cargo test --features sqlite-state
- cargo +1.83.0-x86_64-pc-windows-msvc build --release
- wsl -d Ubuntu-20.04 -- bash -lc "cd /mnt/c/Users/adyba/openclaw-rust && CARGO_TARGET_DIR=target-linux-ubuntu20 /root/.cargo/bin/cargo +1.83.0 check"
- wsl -d Ubuntu-20.04 -- bash -lc "cd /mnt/c/Users/adyba/openclaw-rust && CARGO_TARGET_DIR=target-linux-ubuntu20 /root/.cargo/bin/cargo +1.83.0 test --no-run"
- wsl -d Ubuntu-20.04 -- bash -lc "cd /mnt/c/Users/adyba/openclaw-rust && CARGO_TARGET_DIR=target-linux-ubuntu20 /root/.cargo/bin/cargo +1.83.0 build --release"

## Notes
- CP0 parity wrapper script requires GNU linker setup providing -lgcc_eh. This workstation validation used the full MSVC and Ubuntu matrix.
