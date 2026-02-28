# OpenClaw Rust v1.7.12

## Highlights
- Expanded Telegram auth operator flow for phone-first browser-session handoff:
  - Added `/auth status [provider] [account]` for provider/account OAuth state checks.
  - Added `/auth bridge` diagnostics for configured bridge candidate `/health` probes.
  - Added `/auth wait ... --timeout <seconds>` / `--timeout-ms` for long-lived browser auth waits.
- Added first-class Telegram TTS command surface and playable media delivery:
  - Added `/tts status|providers|provider|on|off|speak` runtime controls.
  - Added multipart Telegram upload path (`sendVoice`/`sendAudio`) so `tts.convert` output is sent as real audio attachments.
  - Added optional automatic Telegram TTS audio for assistant replies when runtime TTS is enabled.
- Hardened gateway `tts.convert` behavior:
  - Added explicit `outputFormat` support (`mp3`, `opus`, `wav`) plus strict validation.
  - Added `requireRealAudio` to fail fast when real synthesis is required but only simulated fallback is available.
  - Added deterministic WAV synthesis bytes for simulated mode to keep Telegram playback reliable.

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
