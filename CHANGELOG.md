# Changelog

## Unreleased

### Highlights
- No unreleased changes.

## v1.7.6 - 2026-02-26

### Highlights
- Hardened official website bridge routing for guest providers under mixed auth states:
  - Added explicit bridge-mode matching and loopback endpoint prioritization for `zai`, `qwen-portal`, and `inception`.
  - Added support for upstream `x-actual-status-code` override handling so HTTP-200 transport envelopes with effective 4xx/5xx statuses are treated correctly.
- Improved provider runtime fallback decisions in `src/gateway.rs`:
  - Guest bridge providers can now still attempt website bridge paths when stale/non-functional API keys are configured.
  - Loopback bridge candidates (`http://127.0.0.1:43010/...`) are auto-appended for guest bridge providers when applicable.
  - Direct provider request errors now include website bridge fallback failure context for faster debugging.
- Updated official Inception/Mercury bridge flow in `src/website_bridge.rs`:
  - Added direct completion-first strategy via `/api/v1/chat/completions` and `/api/chat/completions`.
  - Kept legacy chat-create completion flow as fallback when direct mode fails.
  - Relaxed auth token dependency for guest mode where endpoint behavior permits keyless completion.
- Extended Qwen guest bridge handling in `src/website_bridge.rs`:
  - Added v2 chat/create + completion path support with robust chat ID extraction from multiple response shapes.
  - Preserved v1 auth-backed fallback path for compatibility.
- Added/updated focused bridge and provider tests covering stale-key fallback, loopback endpoint detection, and direct-completion behavior.
- Added cross-platform test compatibility fix for secret echo assertions in `src/tool_runtime.rs`.

### Validation
- `cargo +1.83.0-x86_64-pc-windows-gnu fmt --all -- --check`
- `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-gnu test`
- `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state"`
- `./scripts/parity/run-cp6-gate.ps1`
- `./scripts/parity/run-cp0-gate.ps1`
- Ubuntu 20.04 (WSL):
  - `cargo +1.83.0 fmt --all -- --check`
  - `cargo +1.83.0 clippy --all-targets -- -D warnings`
  - `cargo +1.83.0 test`
  - `cargo +1.83.0 build --release`
- Docker parity smoke:
  - `./scripts/run-docker-parity-smoke.ps1`

## v1.7.2 - 2026-02-25

### Highlights
- Added table (8) edge feature implementations (excluding autonomous self-forking) in `src/gateway.rs` and `src/gateway_server.rs`:
  - `edge.identity.trust.status`
  - `edge.personality.profile`
  - `edge.handoff.plan`
  - `edge.marketplace.revenue.preview`
  - `edge.finetune.cluster.plan`
  - `edge.alignment.evaluate`
  - `edge.quantum.status`
  - `edge.collaboration.plan`
- Added deterministic identity/reputation/trust synthesis helpers for decentralized trust snapshots and route summaries.
- Added Telegram OAuth control command support in `src/telegram_bridge.rs`:
  - `/auth providers`
  - `/auth start <provider> [account] [--force]`
  - `/auth wait <provider> [session_id] [account]`
  - `/auth wait session <session_id> [account]`
- Extended gateway and Telegram tests for the new edge methods and `/auth` command parsing.
- Bumped package version to `1.7.2`.

### Validation
- `cargo +1.83.0-x86_64-pc-windows-gnu fmt --all -- --check`
- `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-gnu test`
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state"`
- `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- `wsl -d Ubuntu-20.04 -- bash -lc 'source $HOME/.cargo/env && cd /mnt/c/Users/adyba/openclaw-rust && CARGO_TARGET_DIR=target-linux-ubuntu20 cargo +1.83.0 build --release'`
- `./scripts/run-docker-parity-smoke.ps1`
- Ubuntu 20.04 RSS probe under active RPC traffic:
  - `MAX_RSS_KB=15744`
  - `MAX_RSS_MB=15.38`

## v1.7.1 - 2026-02-25

### Highlights
- Added Telegram model control command support in `src/telegram_bridge.rs`:
  - `/model list`
  - `/model list <provider>`
  - `/model <provider>/<model>`
  - `/model <provider> <model>`
- Added Telegram provider key patch command:
  - `/set api key <provider> <key>`
  - Applies hashed/base-protected config updates via `config.get` + `config.patch` into `models.providers.<provider>.apiKey`.
- Added catalog-aware command UX for Telegram model selection (provider/model validation and custom override hinting when model IDs are not present in current catalog).
- Extended provider alias normalization in `src/gateway.rs` and Telegram command parsing for key models.dev variants:
  - `fireworks-ai`, `moonshotai`, `moonshotai-cn`, `novita-ai`, `opencode-go`, `kimi-for-coding`, `inference`.
- Normalized `opencode_free`/`opencodefree` aliases to canonical runtime provider `opencode`.

### Validation
- `cargo +1.83.0-x86_64-pc-windows-gnu fmt --all -- --check`
- `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-gnu test`
- `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state"`
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu build --release --features sqlite-state"`
- `./scripts/parity/run-cp0-gate.ps1`
- `./scripts/parity/run-cp6-gate.ps1`
- Ubuntu 20.04 (WSL): `cargo +1.83.0 check`, `cargo +1.83.0 test --no-run`, `cargo +1.83.0 build --release`
- Docker parity smoke attempted via `./scripts/run-docker-parity-smoke.ps1`, but Docker engine was not reachable on this workstation (`//./pipe/dockerDesktopLinuxEngine` not found).

## v1.6.6 - 2026-02-24

### Highlights
- Added profile-aware core/edge runtime defaults:
  - `runtime.profile` now supports `core` and `edge`.
  - TTS fallback order now respects profile defaults (core keeps offline `kittentts` opt-in; edge enables `kittentts` fallback by default).
  - `tts.status` now reports `runtimeProfile` and offline voice recommendation metadata.
- Added configurable self-healing policy controls for `agent` runtime retries:
  - `runtime.selfHealing.enabled`
  - `runtime.selfHealing.maxAttempts`
  - `runtime.selfHealing.backoffMs`
  - plus env overrides: `OPENCLAW_RS_AGENT_SELF_HEAL_ENABLED`, `OPENCLAW_RS_AGENT_SELF_HEAL_MAX_ATTEMPTS`, `OPENCLAW_RS_AGENT_SELF_HEAL_BACKOFF_MS`.
- Extended self-healing telemetry to include policy metadata under `runtime.selfHealing`:
  - `profile`, `maxAttempts`, `backoffMs`, `enabled`, `recovered`, `attempts`.
- Added/updated test coverage:
  - `dispatcher_tts_status_runtime_profile_controls_offline_defaults`
  - extended `dispatcher_agent_runtime_self_heals_with_fallback_provider_retry` assertions for policy metadata.

### Validation
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- `cargo test --features sqlite-state`
- `./scripts/parity/run-cp0-gate.ps1`
- `cargo +1.83.0-x86_64-pc-windows-msvc build --release`
- `wsl -d Ubuntu-20.04 -- bash -lc 'cd /mnt/c/Users/adyba/openclaw-rust && CARGO_TARGET_DIR=target-linux-ubuntu20 /root/.cargo/bin/cargo +1.83.0 check'`
- `wsl -d Ubuntu-20.04 -- bash -lc 'cd /mnt/c/Users/adyba/openclaw-rust && CARGO_TARGET_DIR=target-linux-ubuntu20 /root/.cargo/bin/cargo +1.83.0 test --no-run'`
- `wsl -d Ubuntu-20.04 -- bash -lc 'cd /mnt/c/Users/adyba/openclaw-rust && CARGO_TARGET_DIR=target-linux-ubuntu20 /root/.cargo/bin/cargo +1.83.0 build --release'`

## v1.6.5 - 2026-02-24

### Highlights
- Added light self-healing runtime retry for `agent` turns:
  - When the primary provider execution fails, the dispatcher now attempts bounded fallback-provider retries.
  - Added structured runtime telemetry under `runtime.selfHealing` (`enabled`, `recovered`, `attempts`).
  - Added recovery-path test coverage (`dispatcher_agent_runtime_self_heals_with_fallback_provider_retry`).
- Added offline voice expansion for TTS:
  - Added `kittentts` as a first-class TTS provider option (`tts.setProvider`, `tts.providers`, `tts.status`).
  - Added lazy local-binary detection via `OPENCLAW_RS_KITTENTTS_BIN` (+ optional args via `OPENCLAW_RS_KITTENTTS_ARGS`).
  - Kept graceful fallback to simulated output when local binary is unavailable.
- Added core/edge dual-track planning artifacts:
  - `CORE_EDGE_RELEASE_PLAN_TABLE3_TABLE4.md`
  - `.github/ISSUE_CORE_EDGE_RELEASE_PLAN.md`
- Added dual-tag release bundles:
  - `dist/release-v1.6.5-core/`
  - `dist/release-v1.6.5-edge/`

### Validation
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- `cargo test --features sqlite-state`
- `cargo +1.83.0-x86_64-pc-windows-msvc build --release`
- `wsl -d Ubuntu-20.04 -- bash -lc 'cd /mnt/c/Users/adyba/openclaw-rust && CARGO_TARGET_DIR=target-linux-ubuntu20 /root/.cargo/bin/cargo +1.83.0 check'`
- `wsl -d Ubuntu-20.04 -- bash -lc 'cd /mnt/c/Users/adyba/openclaw-rust && CARGO_TARGET_DIR=target-linux-ubuntu20 /root/.cargo/bin/cargo +1.83.0 test --no-run'`
- `wsl -d Ubuntu-20.04 -- bash -lc 'cd /mnt/c/Users/adyba/openclaw-rust && CARGO_TARGET_DIR=target-linux-ubuntu20 /root/.cargo/bin/cargo +1.83.0 build --release'`
- `cargo run -- doctor --non-interactive --json`
- `cargo run -- security audit --deep --json`
- Notes:
  - `scripts/parity/run-cp0-gate.ps1` was executed and reached the replay suite, but failed on this workstation due missing GNU linker runtime (`-lgcc_eh`). Release validation used the full MSVC + Ubuntu 20.04 matrix and parity corpus tests.

## v1.6.4 - 2026-02-22

### Highlights
- Added `src/persistent_memory.rs` implementing a Rust-native `zvec`-style vector memory backend with persistent disk snapshots, bounded retention, and cosine top-k recall.
- Added a Rust-native `graphlite`-style graph memory backend (session/concept nodes + mention/co-occurrence edges) with synthesized fact recall.
- Integrated persistent memory into `agent` runtime execution:
  - User and assistant turns are ingested into memory stores.
  - Memory recall context is injected into the provider prompt path as a bounded system message.
- Added memory runtime telemetry to `health` and `status` RPC payloads.
- Added config-driven memory runtime parsing from gateway config (`memory.enabled`, `memory.zvecStorePath`, `memory.graphStorePath`, `memory.maxEntries`, `memory.recallTopK`, `memory.recallMinScore`).
- Bumped runtime/tooling contracts to `v1.6.4` (`Cargo.toml`, `wit/tool.wit`, wasm registry WIT test fixture string).

### Validation
- `cargo fmt --all`
- `cargo check`
- `cargo test persistent_memory -- --nocapture`
- `cargo test cli_dispatch_rpc_status_returns_runtime_payload`
- `cargo test dispatcher_chat_methods_follow_parity_contract`
- `cargo clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-msvc check`
- `cargo +1.83.0-x86_64-pc-windows-msvc test --no-run`
- `cargo +1.83.0-x86_64-pc-windows-msvc build --release`
- `cargo run -- gateway --json status`
- `cargo run -- agent --message "memory integration smoke test" --session-key main --idempotency-key v164-smoke --json`

## v1.6.3 - 2026-02-22

### Highlights
- Added upstream-style security CLI parity surface: `openclaw-agent-rs security audit` with `--deep`, `--fix`, and `--json`.
- Added native security audit report model (`summary`, `findings`, optional deep gateway probe block) plus text/JSON rendering parity in CLI output.
- Added deterministic safe-fix flow for common foot-guns (`gateway.server.auth_mode=none`, broad group activation, empty command allow/deny lists) and best-effort permission tightening actions.
- Added filesystem-focused audit checks (config/state/quarantine existence, type mismatches, symlink warnings, unix permission findings).
- Extended CP7 parity gate fixtures to include the new security-audit CLI parsing path.
- Bumped runtime/tooling contracts to `v1.6.3` (`Cargo.toml`, `wit/tool.wit`, wasm registry WIT test fixture string).

### Validation
- `cargo fmt --all`
- `cargo check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `cargo build --release`
- `cargo run -- doctor --non-interactive --json`
- `cargo run -- security audit --json`
- `cargo run -- security audit --deep --json`
- `cargo test --features sqlite-state`
- `cargo clippy --all-targets --features sqlite-state -- -D warnings`
- `cargo build --release --features sqlite-state`
- `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\parity\run-cp7-gate.ps1 -Toolchain 1.83.0-x86_64-pc-windows-msvc`
- `wsl -d Ubuntu bash -lc 'source $HOME/.cargo/env && cd /mnt/c/Users/adyba/openclaw-rust && CARGO_TARGET_DIR=target-linux cargo +1.83.0 check'`
- `wsl -d Ubuntu bash -lc 'source $HOME/.cargo/env && cd /mnt/c/Users/adyba/openclaw-rust && CARGO_TARGET_DIR=target-linux cargo +1.83.0 test --no-run'`
- `wsl -d Ubuntu bash -lc 'source $HOME/.cargo/env && cd /mnt/c/Users/adyba/openclaw-rust && CARGO_TARGET_DIR=target-linux cargo +1.83.0 build --release'`
- `docker run --rm ubuntu:20.04 bash -lc 'cat /etc/os-release'`
- `docker run --rm -v "C:/Users/adyba/openclaw-rust:/workspace" -w /workspace ubuntu:20.04 bash -lc '... CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=target-docker cargo +1.83.0 build --release -q && ./target-docker/release/openclaw-agent-rs doctor --non-interactive --json'`
- `docker run --rm -v "C:/Users/adyba/openclaw-rust:/workspace" -w /workspace ubuntu:20.04 bash -lc '... CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=target-docker cargo +1.83.0 test --no-run -q'`
- Notes:
  - Initial docker release build attempt without constrained jobs exited with `SIGKILL` (container memory pressure); retry with `CARGO_BUILD_JOBS=1` succeeded.
  - Docker `cargo test --no-run -q` exceeded the 30-minute command timeout on this workstation.

## v1.6.2 - 2026-02-22

### Highlights
- Replaced wasm stub execution with a real `wasmtime` runtime path (`src/wasm_runtime.rs`) while preserving policy-level fuel/memory/capability enforcement from `security.tool_runtime_policy.wasm`.
- Added dynamic WIT tool loading + schema generation (`src/tool_registry.rs`) and wired `wasm` actions for runtime registry listing and schema retrieval.
- Extended credential policy with `secret_names` and stronger bidirectional leak redaction across request/response paths (`src/security/credential_injector.rs` + tool runtime hooks).
- Expanded SafetyLayer integration from truncation-only to layered input/output scanning, sanitization, and review/block escalation (`src/security/safety_layer.rs`, `src/security/mod.rs`, `src/tool_runtime.rs`).
- Added new `[security.wasm]` config section with `tool_runtime_mode = "wasm_sandbox"` and WIT registry controls, synchronized into runtime policy (`src/config.rs`, `openclaw-rs.example.toml`, `src/gateway.rs`).
- Added `doctor` wasm checks for runtime mode, module/WIT roots, and optional wasmtime CLI visibility (`src/main.rs`).
- Bumped runtime/tooling contracts to `v1.6.2` (`Cargo.toml`, `wit/tool.wit`), plus new wasm runtime tests.

### Validation
- `cargo fmt --all`
- `cargo check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `cargo build --release`
- `cargo run -- doctor --non-interactive --json`
- `wsl -d Ubuntu bash -lc 'source $HOME/.cargo/env && cd /mnt/c/Users/adyba/openclaw-rust && CARGO_TARGET_DIR=target-linux cargo +1.83.0 check'`
- `wsl -d Ubuntu bash -lc 'source $HOME/.cargo/env && cd /mnt/c/Users/adyba/openclaw-rust && CARGO_TARGET_DIR=target-linux cargo +1.83.0 test --no-run'`
- `wsl -d Ubuntu bash -lc 'source $HOME/.cargo/env && cd /mnt/c/Users/adyba/openclaw-rust && CARGO_TARGET_DIR=target-linux cargo +1.83.0 build --release'`
- `docker run --rm ubuntu:20.04 bash -lc 'cat /etc/os-release'`
- `docker run --rm -v "C:/Users/adyba/openclaw-rust:/workspace" -w /workspace ubuntu:20.04 bash -lc '... CARGO_TARGET_DIR=target-docker cargo +1.83.0 check -q'`
- `docker run --rm -v "C:/Users/adyba/openclaw-rust:/workspace" -w /workspace ubuntu:20.04 bash -lc '... CARGO_TARGET_DIR=target-docker cargo +1.83.0 build --release -q'`
- `docker run --rm -v "C:/Users/adyba/openclaw-rust:/workspace" -w /workspace ubuntu:20.04 bash -lc './target-docker/release/openclaw-agent-rs doctor --non-interactive --json'`
- Note: `docker ... cargo test --no-run` on Ubuntu 20.04 hit container memory limits (`SIGKILL`) on this workstation; build/check/runtime smoke still completed.

## v1.5.2 - 2026-02-22

### Highlights
- Integrated IronClaw-inspired `wasm` runtime surface with capability-gated sandbox inspection/execute flow and policy-backed defaults under `security.tool_runtime_policy.wasm`.
- Added WIT tool interface scaffold at `wit/tool.wit` for contract-first portable tool hosting.
- Added runtime credential injection allowlist + bidirectional leak detection/redaction pipeline (`security.tool_runtime_policy.credentials`) and wired it through `exec`/`process` output handling.
- Added layered safety helper plumbing for defender evaluation and runtime output truncation/sanitization controls (`security.tool_runtime_policy.safety`).
- Added routines/orchestrator runtime surface via the new `routines` tool family with upsert/list/run/history actions (`security.tool_runtime_policy.routines`).
- Fixed workstation MinGW helper script to auto-detect valid toolchains (including WinGet WinLibs) instead of relying on hardcoded user-specific paths.

### Validation
- `cargo +1.83.0-x86_64-pc-windows-gnu fmt --all -- --check`
- `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-gnu test`
- `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- `.\scripts\with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets --features sqlite-state -- -D warnings"`
- `.\scripts\with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state"`
- `.\scripts\with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu build --release --features sqlite-state"`

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
