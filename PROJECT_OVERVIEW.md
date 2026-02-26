# OpenClaw Rust Project Overview

Version: v1.7.6  
Last updated: 2026-02-26

## 1. Project purpose

OpenClaw Rust is a full Rust runtime and gateway implementation designed to replace the original mixed-language OpenClaw control plane for production deployment. The target profile is:

- Ubuntu 20.04 compatibility for long-lived server deployments.
- Bounded memory and queue behavior suitable for low-resource hosts.
- Security-first runtime controls (prompt/command/policy/attestation/VirusTotal).
- Provider-flexible agent execution through OpenAI-compatible APIs and website bridge fallbacks.

The current repo status tracks this as a completed parity program in the local audit surface (`OPENCLAW_FEATURE_AUDIT.md`).

## 2. System architecture

Top-level runtime layers:

1. Gateway and RPC surface (`src/gateway.rs`, `src/gateway_server.rs`)
2. Bridge and scheduler path (`src/bridge.rs`, `src/scheduler.rs`)
3. Agent/provider runtime (`src/gateway.rs` provider resolution + tool loop)
4. Tool runtime host (`src/tool_runtime.rs`)
5. Security pipeline (`src/security/*`)
6. Channel runtime and status model (`src/channels/mod.rs`)
7. Telegram native bridge (`src/telegram_bridge.rs`)
8. State and persistence (`src/state.rs`, store-path backed registries)

Runtime entrypoint:

- `src/main.rs` parses CLI/config and starts `AgentRuntime`.
- `src/runtime.rs` selects mode:
  - `bridge_client`: connects as sidecar to external gateway.
  - `standalone_server`: runs Rust-native gateway server and control surface.

## 3. Core runtime modes

### 3.1 Bridge client mode

Used when Rust is attached to another gateway transport layer.

- Maintains websocket session to configured `gateway.url`.
- Handles action events, scheduling, decisioning, and RPC response logic.
- Preserves idempotency and bounded per-session work behavior.

### 3.2 Standalone server mode

Used for full Rust-first deployment.

- Exposes gateway-compatible websocket RPC surface.
- Supports role/scope authorization.
- Supports control HTTP endpoints (`/health`, `/status`, `/rpc/methods`, `POST /rpc`).
- Runs periodic tick/cutover cron behavior.
- Now includes native Telegram bridge worker lifecycle in runtime startup/shutdown.

## 4. Request/action lifecycle

1. Inbound payload/event arrives over gateway or webhook.
2. Session key and delivery context are resolved.
3. Scheduler enforces per-session FIFO policy and queue strategy (`followup`, `steer`, `collect`).
4. Defender engine scores risk using prompt, command, host, loop, and policy signals.
5. Decision outcome is emitted (`allow` / `review` / `block`) with rationale metadata.
6. Allowed agent requests execute against configured provider runtime.
7. Assistant/tool outputs are persisted to session history and usage structures.
8. Channel adapters or bridge workers send outbound response when applicable.

## 5. Security model

Implemented security controls include:

- Prompt-injection scoring (`src/security/prompt_guard.rs`)
- Destructive command detection (`src/security/command_guard.rs`)
- Host integrity file monitoring (`src/security/host_guard.rs`)
- Tool policy profile and per-provider policy overlays (`src/security/tool_policy.rs`)
- Tool-loop repetition detection with warning/critical thresholds (`src/security/tool_loop.rs`)
- Policy bundle signature verification and key rotation support (`src/security/policy_bundle.rs`)
- Binary/report attestation checks (`src/security/attestation.rs`)
- VirusTotal file/url intelligence integration (`src/security/virustotal.rs`)

Security goals:

- Reject high-risk actions early.
- Keep policy deterministic and auditable.
- Preserve bounded state for loop/tool history tracking.

## 6. Provider and model runtime

The agent runtime supports OpenAI-compatible `chat/completions` flows and provider-specific shaping where required.

Coverage pattern:

- Direct official providers (OpenAI, Anthropic mappings, etc.)
- Cloud routers/aggregators (for example OpenRouter-style endpoints)
- Local/self-hosted OpenAI-compatible stacks (Ollama-compatible style endpoints via config)
- Bridge-capable website fallback modes (`website-openai-bridge`, `website-bridge`, `official-website-bridge`)

Built-in setup defaults include keyless onboarding candidates (OpenCode Zen promo models) and Zhipu model defaults, with alias normalization for major provider naming variants.

Detailed provider matrix:

- `PROVIDER_SUPPORT_MATRIX.md`

## 7. Channel and transport behavior

Runtime channel model includes:

- Canonical channel-id normalization and alias handling.
- Account/runtime snapshot hydration from events.
- Mention and group-activation semantics.
- Outbound retry/backoff and chunking helpers.
- `channels.status`/`channels.logout` parity-shaped payloads.

Wave coverage includes mainstream and extended adapters (telegram/discord/slack/signal/webchat/whatsapp and additional wave-2/3/4 channels documented in audit and tests).

## 8. Telegram native bridge

`src/telegram_bridge.rs` provides a Rust-native Telegram integration path in standalone mode:

- Pulls live channel/model settings from `config.get`.
- Polls Telegram Bot API (`getUpdates`) and tracks durable offsets.
- Applies DM/group gating and allowlist policy.
- Runs `agent` + `agent.wait` and extracts assistant output from history.
- Emits `telegram.status`, inbound, and outbound runtime events.
- Uses provider fallback candidates when the base run fails.
- Supports operator control commands in Telegram:
  - `/model list [provider]` and `/model <provider>/<model>`
  - `/set api key <provider> <key>` (config patch to `models.providers.<provider>.apiKey`)
  - `/auth providers`, `/auth start <provider> [account]`, and `/auth wait ...` for OAuth provider login handoff

This closes the previously observed "configured but no reply" failure mode where Telegram was not actually running as a native runtime worker.

## 9. Persistence and durability

Disk-backed runtime state is supported through configurable store paths for:

- Sessions and usage snapshots
- Channel runtime/account status
- Device/node pair registries
- Config, wizard, and web-login control-plane state
- Idempotency caches
- Telegram update offsets (primary + legacy fallback path support)

SQLite-state mode is also supported for targeted state durability scenarios.

## 10. Performance and memory strategy

The runtime is designed for constrained hosts:

- Bounded global queue (`runtime.max_queue`)
- Bounded worker concurrency (`runtime.worker_concurrency`)
- Single active task per session with deterministic queue mode
- Bounded loop/policy/idempotency registries
- Optional memory sampling telemetry (`runtime.memory_sample_secs`)
- Slow-consumer drop semantics in event fanout to avoid unbounded growth

## 11. Testing and validation model

Primary validation gates:

- `cargo +1.83.0-x86_64-pc-windows-gnu fmt --all -- --check`
- `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-gnu test`
- `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- Optional sqlite-state and CP gate scripts under `scripts/parity/*`
- Ubuntu/WSL build path via `scripts/build-ubuntu20.sh`

Evidence surfaces:

- `OPENCLAW_FEATURE_AUDIT.md`
- `parity/generated/*`
- `CHANGELOG.md`

## 12. Release layout and artifacts

Release bundles are assembled under:

- `dist/release-vX.Y.Z/`

Typical artifact set:

- `openclaw-agent-rs-windows-x86_64.exe`
- `openclaw-agent-rs-linux-x86_64`
- `openclaw-agent-rs-vX.Y.Z-windows-x86_64.zip`
- `openclaw-agent-rs-vX.Y.Z-linux-x86_64.tar.gz`
- `SHA256SUMS.txt`
- `RELEASE_NOTES_vX.Y.Z.md`

## 13. Operational references

- Main quickstart and runtime config: `README.md`
- Feature parity and status: `OPENCLAW_FEATURE_AUDIT.md`
- Provider endpoint matrix: `PROVIDER_SUPPORT_MATRIX.md`
- Migration and critical path: `MIGRATION_PLAN.md`, `RUST_PARITY_CRITICAL_PATH.md`

## 14. Known scope notes

- Voice workflows remain optional and can be disabled for lean deployments.
- Website bridge behavior depends on external provider website/API availability and policy changes.
- For production hardening, keep gateway auth configured and avoid running with unrestricted tool profiles.

