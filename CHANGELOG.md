# Changelog

## Unreleased

### Highlights
- No unreleased changes.

## v1.0.2 - 2026-02-22

### Highlights
- Added native Telegram bridge runtime execution in standalone mode so Telegram is truly online (`running=true`) and can reply through Rust without external helper processes.
- Fixed standalone runtime shutdown flow for the bridge task lifecycle (clean abort/join handling).
- Added full technical system document `PROJECT_OVERVIEW.md` covering architecture, data flow, security, provider runtime, persistence, performance constraints, and operational release layout.
- Rebuilt and repackaged release artifacts for Windows and Ubuntu 20.04 compatible Linux bundles under `dist/release-v1.0.2`.

### Validation
- `cargo +1.83.0-x86_64-pc-windows-gnu fmt --all -- --check`
- `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-gnu test`
- `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- `wsl -d Ubuntu-20.04 bash -lc "cd /mnt/c/users/ady/documents/openclaw-rust && rustup toolchain install 1.83.0 && CARGO_TARGET_DIR=target-linux cargo +1.83.0 build --release"`

## v1.0.1 - 2026-02-22

### Highlights
- Added official `chat.z.ai` guest-bridge flow support in the website bridge runtime path.
- Expanded OpenAI-compatible provider presets and alias normalization coverage for local runtimes, cloud providers, and router-style endpoints.
- Kept keyless OpenCode Zen free-model runtime defaults (`glm-5-free`, `kimi-k2.5-free`, `minimax-m2.5-free`) available for first-run onboarding.

### Validation
- `cargo +1.83.0-x86_64-pc-windows-gnu fmt --all -- --check`
- `cargo +1.83.0-x86_64-pc-windows-gnu test`
- `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- `.\scripts\with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state"`
- `.\scripts\with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets --features sqlite-state -- -D warnings"`
- `.\scripts\with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu build --release --features sqlite-state"`
- `wsl -d Ubuntu-20.04 ./scripts/build-ubuntu20.sh`

## v1.0.0 - 2026-02-22

### Highlights
- Completed Rust end-to-end control-plane and session parity coverage for the OpenClaw gateway surface (including `agent`, `sessions.*`, `chat.*`, `models.*`, `agents.*`, `exec.*`, `node.*`, `device.*`, `cron.*`, `skills.*`, and standalone server control HTTP).
- Added OpenAI-compatible provider runtime parity with configurable auth headers, request defaults, nested provider options, and provider alias normalization.
- Added website-bridge runtime support for keyless/official web fallback flows:
  - API modes: `website-openai-bridge`, `website-bridge`, `official-website-bridge`.
  - Configurable `websiteUrl` health probe and `bridgeBaseUrls` candidate failover chain.
  - OpenAI-compatible request shaping with optional auth headers and tool payload support.
- Added setup-ready free-tier defaults for OpenCode Zen models:
  - `glm-5-free`
  - `kimi-k2.5-free`
  - `minimax-m2.5-free`
- Preserved security controls and hardening behavior, including prompt-injection scoring, command guardrails, policy-bundle verification, host attestation checks, and loop detection escalation.

### Validation
- `cargo +1.83.0-x86_64-pc-windows-gnu fmt --all -- --check`
- `cargo +1.83.0-x86_64-pc-windows-gnu test`
- `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- `.\scripts\with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state"`
- `.\scripts\with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets --features sqlite-state -- -D warnings"`
- `.\scripts\with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu build --release --features sqlite-state"`
- `.\scripts\parity\run-replay-corpus.ps1`
- `.\scripts\parity\run-cp8-gate.ps1`
- `.\scripts\parity\run-cp9-gate.ps1`
- Live bridge smoke tests against OpenCode Zen free models:
  - `gateway::tests::live_openai_compatible_opencode_smoke_when_credentials_are_configured` with `glm-5-free`
  - Same test with `kimi-k2.5-free`
  - Same test with `minimax-m2.5-free`
