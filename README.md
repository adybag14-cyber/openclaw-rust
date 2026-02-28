# OpenClaw Agent (Rust)

This directory contains the Rust rewrite foundation for the OpenClaw runtime.

Minimum supported Rust version: `1.83`.

## Project overview

For a full architecture and subsystem deep dive (runtime layers, security model,
provider runtime, persistence, performance strategy, and release layout), see:

- `PROJECT_OVERVIEW.md`

## Current parity status (February 28, 2026)

- End-to-end Rust parity program status: **complete**.
- Feature audit scoreboard: `22 implemented`, `0 partial`, `0 deferred`.
- RPC method-surface parity: `132` Rust methods, `100%` coverage vs upstream base + handlers.
- Runtime audit: blanket dead-code suppression removed; only targeted transcript-entry allowance remains in `tool_runtime` for parity/test inspection fields.
- Memory integrations shipped in the `1.6.6` baseline:
  - Added a native Rust `zvec`-style persistent vector memory engine (`src/persistent_memory.rs`) with bounded on-disk storage and cosine top-k recall.
  - Added a native Rust `graphlite`-style persistent graph memory store (session/concept nodes + mention/co-occurrence edges) with synthesized graph facts for recall.
  - Wired memory ingestion into the live `agent` runtime path for both user turns and assistant outputs.
  - Wired memory recall into agent turn execution as bounded system-context injection before provider completion calls.
  - Added runtime memory telemetry to `gateway status` / `health` responses (`memory.enabled`, entry/node/edge counts, store paths, recall limits).
  - Added config-driven memory tuning under `memory.*` (`enabled`, `zvecStorePath`, `graphStorePath`, `maxEntries`, `recallTopK`, `recallMinScore`).
- Current core/edge release-track additions:
  - Added light self-healing runtime retries for `agent` execution failures with structured `runtime.selfHealing` response telemetry.
  - Added offline voice provider surface for `kittentts` (lazy local-binary mode, optional via `OPENCLAW_RS_KITTENTTS_BIN`).
  - Added profile-aware runtime defaults (`runtime.profile`: `core`/`edge`) used by TTS fallback behavior and self-healing policy.
  - Added configurable self-healing policy controls (`runtime.selfHealing.enabled`, `runtime.selfHealing.maxAttempts`, `runtime.selfHealing.backoffMs`) plus env overrides (`OPENCLAW_RS_AGENT_SELF_HEAL_*`).
  - Added dual-track planning artifacts: `CORE_EDGE_RELEASE_PLAN_TABLE3_TABLE4.md` and issue template `.github/ISSUE_CORE_EDGE_RELEASE_PLAN.md`.
  - Added executable enclave attestation bridge flow for `edge.enclave.prove` via configurable binary runtime (`OPENCLAW_RS_ENCLAVE_ATTEST_BIN`) with persisted proof records exposed on `edge.enclave.status`.
  - Added executable fine-tune trainer flow for `edge.finetune.run` (`dryRun=false`) with bounded trainer timeout/log capture and persisted per-job state exposed through `edge.finetune.status`.
  - Added live mesh runtime probes to `edge.mesh.status` using `mesh.ping` invoke waits, returning probe success/timeout telemetry and failed-peer summaries.
  - Added table-8 edge capability surfaces (excluding autonomous self-forking): decentralized identity trust (`edge.identity.trust.status`), personality engine (`edge.personality.profile`), cross-device handoff planning (`edge.handoff.plan`), marketplace revenue preview (`edge.marketplace.revenue.preview`), cluster finetune planning (`edge.finetune.cluster.plan`), ethical alignment evaluation (`edge.alignment.evaluate`), quantum-safe status (`edge.quantum.status`), and mixed-initiative collaboration planning (`edge.collaboration.plan`).
- Latest full validation matrix:
  - `cargo +1.83.0-x86_64-pc-windows-gnu test` -> `405` passed (`1` ignored)
  - `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state"` -> `409` passed (`1` ignored)
  - `cargo +1.83.0-x86_64-pc-windows-gnu fmt --all -- --check` + `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings` pass
  - Windows release builds pass: `cargo +1.83.0-x86_64-pc-windows-gnu build --release` and `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu build --release --features sqlite-state"`
  - Ubuntu 20.04 WSL release build passes: `wsl -d Ubuntu-20.04 -- bash -lc 'source $HOME/.cargo/env && cd /mnt/c/Users/Ady/Documents/openclaw-rust && CARGO_TARGET_DIR=target-linux-ubuntu20 cargo +1.83.0 build --release'`
  - Docker parity smoke passes with workstation Docker memory profile updated: `./scripts/run-docker-parity-smoke.ps1`
  - Ubuntu 20.04 runtime RSS probe peak (active RPC traffic): `15.38 MB` (`MAX_RSS_KB=15744`)
  - Current release tag: `v1.7.13`

## Implemented runtime coverage

- Native Rust runtime suitable for Ubuntu 20.04 deployment.
- Gateway compatibility bridge over OpenClaw's WebSocket protocol.
- Standalone Rust Gateway WebSocket runtime mode (no TypeScript gateway process).
- Defender pipeline that can block/review suspicious actions before execution.
- VirusTotal lookups (file hash + URL) to add external threat intelligence.
- Host integrity baseline checks for key runtime files.
- Bounded concurrency and queue limits to reduce memory spikes.
- Session FIFO scheduling + decision state tracking + idempotency cache.
- Live OpenAI-compatible agent runtime path with multi-provider endpoint/auth resolution and tool-calling loop execution through the Rust tool runtime.
- Typed session-key parsing (`main`, `direct`, `group`, `channel`, `cron`, `hook`, `node`).
- Typed protocol frame foundation (`req`/`resp`/`event` classification).
- Gateway RPC parity scaffold for `sessions.list`, `sessions.preview`, `sessions.patch`, `sessions.resolve`, `sessions.reset`, `sessions.delete`, `sessions.compact`, `sessions.usage`, `sessions.usage.timeseries`, `sessions.usage.logs`, `sessions.history`, `sessions.send`, and `session.status`.
- Channel adapter scaffold (`telegram`, `whatsapp`, `discord`, `irc`, `slack`, `signal`, `imessage`, `webchat`, `bluebubbles`, `googlechat`, `msteams`, `matrix`, `zalo`, `zalouser`, `feishu`, `mattermost`, `line`, `nextcloud-talk`, `nostr`, `tlon`, generic) with wave-1/wave-2/wave-3/wave-4 channel-runtime helpers (chat-type normalization, mention gating, chunking, retry/backoff, alias canonicalization) and event-driven runtime snapshot ingestion for `channels.status`.
- Telegram bridge operator commands for live runtime control:
  - `/model list [provider]` and `/model <provider>/<model>` for session model selection.
  - `/set api key <provider> <key>` for provider credential patching into `models.providers.<provider>.apiKey`.
  - `/auth providers`, `/auth status [provider] [account]`, `/auth bridge`, `/auth start <provider> [account]`, `/auth wait ... [--timeout <seconds>]`, and `/auth complete ...` for OAuth login handoff diagnostics and callback completion.
  - `/tts status|providers|provider|on|off|speak` for Telegram-native TTS runtime control and audio clip delivery.
- Runtime defender hardening extensions including EDR telemetry ingestion and runtime binary attestation checks.

Rust is now the primary runtime implementation for required parity surfaces; this
repository tracks post-parity optimization and hardening work rather than parity
bootstrapping.

## Ubuntu 20.04 setup

```bash
curl https://sh.rustup.rs -sSf | sh -s -- -y
source "$HOME/.cargo/env"

cd rust-agent
cp openclaw-rs.example.toml openclaw-rs.toml

# Optional: set your token + VT key
export OPENCLAW_RS_GATEWAY_TOKEN="..."
export OPENCLAW_RS_VT_API_KEY="..."

cargo run --release -- --config ./openclaw-rs.toml
```

CP7 CLI parity quick checks:

```bash
cargo run -- doctor --non-interactive
cargo run -- security audit --json
cargo run -- security audit --deep --json
cargo run -- status --json
cargo run -- health --json
cargo run -- tools catalog --json
cargo run -- gateway call --method tools.catalog --params '{"includePlugins":false}' --json
cargo run -- agent --message "status check" --wait --json
cargo run -- message send --to "+15551234567" --message "hello" --channel telegram --json
cargo run -- nodes list --json
cargo run -- sessions list --limit 5 --json
```

## Build + service on Ubuntu 20.04

```bash
# Build with pinned toolchain
bash ./scripts/build-ubuntu20.sh

# Install as user service
mkdir -p ~/.config/systemd/user
cp ./deploy/openclaw-agent-rs.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now openclaw-agent-rs.service
systemctl --user status openclaw-agent-rs.service
```

## Default runtime behavior

- Runtime mode is selected by `gateway.runtime_mode`:
  - `bridge_client`: connects to `gateway.url` as a defender sidecar.
  - `standalone_server`: listens on `gateway.server.bind` and serves gateway RPCs directly.
- In `bridge_client` mode, it sends a `connect` frame as `openclaw-agent-rs`.
- Responds to core session RPCs (`sessions.list`, `sessions.preview`, `sessions.patch`, `sessions.resolve`, `sessions.reset`, `sessions.delete`, `sessions.compact`, `sessions.usage`, `sessions.usage.timeseries`, `sessions.usage.logs`, `sessions.history`, `sessions.send`, `session.status`) with typed `resp` frames.
- Supports list filtering knobs on `sessions.list` (`includeGlobal`, `includeUnknown`, `agentId`, `search`, `label`, `spawnedBy`) plus optional hint fields (`displayName`, `derivedTitle`, `lastMessagePreview`) when `includeDerivedTitles`/`includeLastMessage` are set.
- Supports `sessions.patch` via either `key` or `sessionKey` and returns parity-style envelope fields (`ok`, `path`, `key`, `entry`).
- Supports extended `sessions.patch` parity fields (`thinkingLevel`, `verboseLevel`, `reasoningLevel`, `responseUsage`, `elevatedLevel`, `execHost`, `execSecurity`, `execAsk`, `execNode`, `model`, `spawnDepth`) with explicit `null` clear semantics.
- Supports provider-defined catalog ingestion from `models.providers.*.models` and OpenAI-compatible provider resolution (including Cerebras-compatible chat completion payload formatting) for runtime agent execution.
- Supports nested provider runtime options (`models.providers.<id>.options`) for OpenAI-compatible endpoints, including custom auth header names/prefixes, custom request defaults, and full `chat/completions` URLs with query strings; local provider defaults (`ollama`, `vllm`, `litellm`, `lmstudio`, `localai`, `llamacpp`, `tgi`, `gpt4all`, `koboldcpp`, `oobabooga`) can run without API keys.
- Supports website bridge API modes (`website-openai-bridge`, `website-bridge`, `official-website-bridge`) with candidate endpoint failover for official web-model fallback paths and keyless provider startup flows.
- Includes built-in setup-ready model choices for OpenCode Zen free promotions (`glm-5-free`, `kimi-k2.5-free`, `minimax-m2.5-free`), ZhipuAI `glm-5`, Qwen 3.5 variants (`qwen3.5-397b-a17b`, `qwen3.5-plus`, `qwen3.5-flash`), Mercury 2 (`inception`), and OpenRouter free routing (`google/gemini-2.0-flash-exp:free`, `qwen/qwen3-next-80b-a3b-instruct:free`, `qwen/qwen3-coder:free`, `inception/mercury`), with provider aliases/defaults for `zhipuai`, `qwen-portal`, and `inception`.
- Enforces parity-oriented patch guards for labels and subagent metadata (`label` uniqueness, `spawnedBy`/`spawnDepth` subagent-only and immutable after first set).
- Normalizes/validates patch tuning values to parity-friendly canonical sets (thinking, verbose, reasoning, elevated, and exec policy knobs).
- Supports `sessions.delete` parity envelope fields (`path`, `archived`) and honors `deleteTranscript` to skip transcript archive hints.
- Supports `sessions.compact` parity envelope fields (`path`, `archived`) with archive hints when transcript compaction removes lines.
- Tracks a stable per-session `sessionId` in session metadata, resolves keys by `sessionId` in `sessions.resolve`, and rotates `sessionId` on `sessions.reset`.
- Normalizes alias and short-form session keys (`main`, `discord:group:*`, etc.) to canonical `agent:*` keys across session RPC handlers.
- Aligns reset/compact parity semantics with upstream defaults (`sessions.reset` reason must be `new|reset`; `sessions.compact` defaults to 400 lines and rejects `maxLines < 1`).
- Enforces upstream `sessions.patch.sendPolicy` parity (`allow|deny|null`); legacy `inherit` is rejected at the RPC boundary.
- Adds session list parity hints for delivery metadata (`lastAccountId`, `deliveryContext`) and token freshness (`totalTokensFresh`).
- Extends `sessions.history` lookups to accept `key` aliases and direct `sessionId` lookups.
- Matches upstream patch semantics where `reasoningLevel="off"` and `responseUsage="off"` clear stored overrides.
- Preserves caller-provided key strings in `sessions.preview` results while still resolving canonical aliases internally.
- Tightens session label parity to upstream rules (max 64 chars, over-limit values rejected instead of truncated).
- Applies the same strict label-length validation to `sessions.list` and `sessions.resolve` filters.
- Responds to gateway introspection RPCs (`health`, `status`) with runtime/session metadata.
- Responds to usage RPCs (`usage.status`, `usage.cost`) with Rust-side aggregate usage/cost placeholder summaries.
- Runs due cron jobs automatically in standalone mode with a bounded in-process tick worker (no explicit `cron.run` call required for due schedules).
- Executes cron webhook delivery side effects (`delivery.mode = webhook`) as HTTP POST callbacks with upstream-aligned `finished` event payloads, optional bearer-token auth (`cron.webhookToken`), summary gating, and legacy `notify: true` fallback via `cron.webhook` (with one-time deprecation warning logs).
- Tracks session metadata (`label`, `spawnedBy`) via `sessions.patch` and uses it for filtered `sessions.resolve` lookups.
- Supports `sessions.usage` range inputs (`startDate`, `endDate`) and optional `includeContextWeight` output hints.
- Extends `sessions.usage` response parity with `updatedAt`, `startDate`/`endDate`, totals, action rollups, and aggregate placeholder sections (`messages`, `tools`, `byAgent`, `byChannel`).
- Inspects incoming Gateway frames for actionable payloads (prompt/command/url/file).
- Applies group activation policy (`mention` or `always`) before evaluation for group contexts.
- Schedules one active request per session with configurable queue behavior (`followup`, `steer`, `collect`).
- Evaluates each action with:
  - prompt injection detector,
  - command risk detector,
  - host integrity monitor,
  - VirusTotal lookups (if configured).
- Emits a `security.decision` event with allow/review/block and reasons.
- Includes session routing hints (`sessionKind`, `chatType`, `wasMentioned`, `replyBack`, `deliveryContext`) in decision events when available.
- Writes blocked actions to `security.quarantine_dir`.

## Config knobs for performance and safety

- `runtime.worker_concurrency`: upper bound for simultaneous evaluations.
- `runtime.max_queue`: bounded work queue.
- `runtime.session_queue_mode`: session queue behavior (`followup`, `steer`, `collect`).
- `runtime.group_activation_mode`: group activation gating (`mention`, `always`).
- `runtime.eval_timeout_ms`: fail-safe timeout per decision.
- `runtime.memory_sample_secs`: periodic RSS logging cadence on Linux.
- `runtime.idempotency_ttl_secs`: duplicate decision cache retention window.
- `runtime.idempotency_max_entries`: cap for idempotency cache footprint.
- `runtime.session_state_path`: JSON state store by default; use `.db/.sqlite/.sqlite3` with `sqlite-state` for SQLite WAL-backed state.
- `security.review_threshold`: minimum risk for "review".
- `security.block_threshold`: minimum risk for "block".
- `security.protect_paths`: files to hash and verify at runtime.
- `security.tool_policies`: per-tool floor action (`allow`, `review`, `block`).
- `security.tool_risk_bonus`: per-tool additive risk scoring.
- `security.channel_risk_bonus`: per-channel additive risk scoring.
- `security.wasm.tool_runtime_mode`: runtime mode (`wasm_sandbox`, `inspection_stub`) for wasm tool execution.
- `security.wasm.wit_root`: dynamic WIT tool discovery root.
- `security.wasm.dynamic_wit_loading`: auto-refresh WIT registry at runtime.
- `security.tool_runtime_policy.profile`: base tool profile (`minimal`, `coding`, `messaging`, `full`).
- `security.tool_runtime_policy.allow` / `security.tool_runtime_policy.deny`: wildcard/group policy filters (`group:fs`, `group:runtime`, etc.).
- `security.tool_runtime_policy.byProvider`: provider/model-specific policy overrides.
- `security.tool_runtime_policy.loop_detection`: loop guard controls (`enabled`, `history_size`, `warning_threshold`, `critical_threshold`).
- `security.tool_runtime_policy.wasm`: wasm execution policy (`enabled`, `module_root`, `wit_root`, `dynamic_wit_loading`, capability map, fuel/memory limits).
- `security.tool_runtime_policy.credentials.secret_names`: configured host secret env names to detect/redact in request/response paths.
- `security.tool_runtime_policy.safety`: layered output safety controls (`enabled`, `sanitize_output`, `max_output_chars`).
- `security.policy_bundle_path`: optional signed JSON policy bundle file to load at startup.
- `security.policy_bundle_key`: HMAC key used to verify the bundle signature.
- `gateway.password`: optional shared-secret password for gateway auth.
- `gateway.runtime_mode`: `bridge_client` or `standalone_server`.
- `gateway.server.bind`: standalone server bind address.
- `gateway.server.http_bind`: optional standalone control HTTP bind address (`/health`, `/status`, `/rpc/methods`, `POST /rpc`).
- `gateway.server.auth_mode`: `auto`, `none`, `token`, or `password`.
- `gateway.server.handshake_timeout_ms`: max connect handshake duration.
- `gateway.server.event_queue_capacity`: per-connection outbound event queue cap.
- `gateway.server.tick_interval_ms`: standalone tick-event cadence and advertised `policy.tickIntervalMs`.
- `gateway.server.reload_interval_secs`: config live-reload polling interval (`0` disables live reload).

## Provider setup examples

For full provider coverage status (built-in defaults vs alias-only/config-required, bridge defaults, OAuth catalog, and endpoint references), see:

- `PROVIDER_SUPPORT_MATRIX.md`

Use `config.patch`/`config.apply` to register OpenAI-compatible providers explicitly:

```json
{
  "models": {
    "providers": {
      "opencode": {
        "api": "website-openai-bridge",
        "baseUrl": "https://opencode.ai/zen/v1",
        "websiteUrl": "https://opencode.ai",
        "bridgeBaseUrls": [
          "https://opencode.ai/zen/v1",
          "https://api.opencode.ai/v1"
        ],
        "allowUnauthenticated": true,
        "apiKey": "${OPENCODE_API_KEY}",
        "models": [
          { "id": "glm-5-free", "name": "GLM-5-Free" },
          { "id": "kimi-k2.5-free", "name": "Kimi K2.5 Free" },
          { "id": "minimax-m2.5-free", "name": "MiniMax M2.5 Free" }
        ]
      },
      "zhipuai": {
        "api": "openai-completions",
        "baseUrl": "https://open.bigmodel.cn/api/paas/v4",
        "apiKey": "${ZHIPUAI_API_KEY}",
        "models": [{ "id": "glm-5", "name": "GLM-5" }]
      },
      "deepinfra": {
        "api": "openai-completions",
        "baseUrl": "https://api.deepinfra.com/v1/openai",
        "apiKey": "${DEEPINFRA_API_KEY}",
        "models": [{ "id": "deepseek-ai/DeepSeek-V3", "name": "DeepSeek V3 (DeepInfra)" }]
      },
      "llamacpp": {
        "api": "openai-completions",
        "baseUrl": "http://127.0.0.1:8080/v1",
        "allowUnauthenticated": true,
        "models": [{ "id": "local-model", "name": "Local llama.cpp" }]
      }
    }
  }
}
```

`apiKey` is optional for providers that expose public/free tiers. When omitted, OpenClaw Rust attempts keyless bridge candidates in priority order.

For Zhipu coding-plan models, use provider `zhipuai-coding` or override `baseUrl` with `https://open.bigmodel.cn/api/coding/paas/v4`.

For additional official website bridges (for example Kimi/Minimax/Zhipu web surfaces), keep `api` in website-bridge mode and configure provider-specific `websiteUrl` plus `bridgeBaseUrls` endpoints exposed by that provider.

Kimi is supported as:

- OpenAI-compatible API runtime (`kimi-coding`) with key/token auth.
- OAuth provider catalog entry (`auth.oauth.*`).
- Website bridge hint surface (`websiteUrl = https://www.kimi.com`) for explicitly configured bridge mode.

Guest/no-login website execution is not assumed for Kimi in runtime defaults; configure authenticated bridge/API flows.

### ChatGPT Browser Session Bridge (Playwright + Puppeteer)

For ChatGPT browser-session usage (OAuth login, no OpenAI API key), run the local bridge:

```bash
node scripts/chatgpt-browser-bridge.mjs
```

Then complete OAuth in Telegram:

```text
/auth start chatgpt
/auth wait chatgpt default
```

When an OpenAI OAuth browser credential is present, runtime bridge candidates include:

- `http://127.0.0.1:43010/v1`
- `http://127.0.0.1:43010`
- `https://chatgpt.com`
- `https://chat.openai.com`

Browser-session model aliases supported for OpenAI provider selection include:

- `gpt-5.2-pro`
- `gpt-5.2-thinking`
- `gpt-5.2-instant`
- `gpt-5.2-auto`
- `gpt-5.2`
- `gpt-5.1`
- `gpt-5-mini`

Bridge-ready provider aliases are normalized for:

- Local/self-hosted: `ollama`, `vllm`, `litellm`, `lmstudio`, `localai`, `llamacpp`, `tgi`, `gpt4all`, `koboldcpp`, `oobabooga`
- Cloud OpenAI-compatible: `groq`, `google`, `deepseek`, `deepinfra`, `mistral`, `fireworks`, `together`, `cerebras`, `siliconflow`, `sambanova`, `novita`, `hyperbolic`, `nebius`, `inference-net`
- Routers/aggregators: `openrouter`, `aimlapi`
- Enterprise/config-driven aliases (set explicit `baseUrl` in config): `azure-openai`, `vertex-ai`, `bedrock`, `cohere`, `xai`, `github-models`, `vercel-ai-gateway`, `shareai`, `bifrost`

## Signed policy bundles

When `security.policy_bundle_path` and `security.policy_bundle_key` are set, the
runtime verifies and applies a signed bundle at startup.

Bundle shape:

```json
{
  "version": 1,
  "bundleId": "ops-policy-2026-02-18",
  "signedAt": "2026-02-18T00:00:00Z",
  "policy": {
    "reviewThreshold": 35,
    "blockThreshold": 65,
    "toolPolicies": { "gateway": "review" }
  },
  "signature": "hex-hmac-sha256"
}
```

Signature rule:

- Compute HMAC-SHA256 over the bundle JSON without the `signature` field.
- Canonicalization sorts object keys recursively before hashing.
- Hex-encode digest as lowercase (or prefix with `sha256:`).

## Planned migration phases

1. Keep existing features through protocol compatibility while moving guardrails to Rust.
2. Move core scheduling/session state to Rust.
3. Move high-throughput channel adapters incrementally behind trait-based drivers.
4. Keep protocol schema stable for macOS/iOS/Android/Web clients during migration.

## Replay Harness (sidecar integration)

The replay harness runs the real bridge + defender engine against fixture frames and
asserts emitted `security.decision` output.

```bash
cargo test replay_harness_with_real_defender -- --nocapture
# or:
bash ./scripts/run-replay-harness.sh
```

## Protocol Corpus Snapshot

The protocol corpus test validates typed frame classification and method-family
mapping against versioned fixtures.

```bash
cargo test protocol_corpus_snapshot_matches_expectations -- --nocapture
# or:
bash ./scripts/run-protocol-corpus.sh
```

## RPC Method Surface Parity Contract

Phase-1 parity tracking now includes an automated upstream-vs-Rust RPC method
surface diff.

```powershell
.\scripts\parity\method-surface-diff.ps1 -Surface both
```

Phase-2 parity tracking includes fixture-driven response/event payload shape checks
for selected upstream RPC handlers:

```powershell
.\scripts\parity\payload-shape-diff.ps1
```

CP0 scoreboard and replay-corpus gate:

```powershell
.\scripts\parity\build-scoreboard.ps1
.\scripts\parity\run-replay-corpus.ps1
.\scripts\parity\run-cp0-gate.ps1 -UpstreamRepoPath ..\openclaw
```

CP1 standalone gateway runtime gate:

```powershell
.\scripts\parity\run-cp1-gate.ps1
```

CP2 session/routing gate (includes fixture duration metrics + artifact logs):

```powershell
.\scripts\parity\run-cp2-gate.ps1
```

CP3 tool-runtime parity gate (`profile/allow/deny/byProvider` + loop guard + transcript/runtime corpus):

```powershell
.\scripts\parity\run-cp3-gate.ps1
```

CP4 channel-runtime wave-1/wave-2/wave-3/wave-4 acceptance + canary gate (registry + normalization + mention/chunk/retry + lifecycle/event parity helpers):

```powershell
.\scripts\parity\run-cp4-gate.ps1
```

CP5 nodes/browser/canvas/device parity gate (runtime invoke + browser proxy + canvas present + pairing/device contract):

```powershell
.\scripts\parity\run-cp5-gate.ps1
```

CP6 model provider/auth/failover gate (provider alias normalization + auth override + runtime failover fixtures):

```powershell
.\scripts\parity\run-cp6-gate.ps1
```

CP7 CLI/control parity gate (`doctor` + `security audit` + gateway/agent/message/nodes/sessions CLI fixtures + control update contract checks):

```powershell
.\scripts\parity\run-cp7-gate.ps1
```

CP8 reliability/security/performance gate (replay + soak + defender regression + benchmark + cutover runbook validation):

```powershell
.\scripts\parity\run-cp8-gate.ps1
```

Current payload corpus coverage: `chat.*`, `tts.*`, `voicewake.*`, `web.login.*`, `update.run`, `send`, `poll`, `sessions.*` envelope/alias flows (including outbound transcript side-effect checks for `send`/`poll`), `browser.request`, `config.*`, `logs.tail`, `cron.*`, `exec.approvals.*`, `exec.approval.*`, and `wizard.*`.

Generated artifacts:

- `parity/PARITY_CONTRACT.md`
- `parity/manifest/PARITY_MANIFEST.v1.json`
- `parity/manifest/scoreboard-baseline.json`
- `parity/generated/upstream-methods.base.json`
- `parity/generated/upstream-methods.handlers.json`
- `parity/generated/rust-methods.json`
- `parity/generated/method-surface-diff.json`
- `parity/generated/parity-scoreboard.json`
- `parity/generated/parity-scoreboard.md`
- `parity/method-surface-report.md`
- `tests/parity/gateway-payload-corpus.json`
- `tests/parity/tool-runtime-corpus.json`
- `parity/generated/cp7/*`
- `parity/generated/cp8/*`
- `parity/CP8_CUTOVER_RUNBOOK.md`

PR automation:

- `.github/workflows/parity-cp0-gate.yml` runs on every PR and publishes parity
  delta summaries in the workflow job summary.

## Windows GNU toolchain helper (SQLite feature)

When using `x86_64-pc-windows-gnu` with `--features sqlite-state`, run through:

```powershell
.\scripts\with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state"
.\scripts\with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets --features sqlite-state -- -D warnings"
.\scripts\with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu build --release --features sqlite-state"
```

## Docker parity smoke

Runs the full Rust validation matrix in Linux (`test`, `clippy`, `release build`,
default + `sqlite-state`):

```bash
bash ./scripts/run-docker-parity-smoke.sh
```

```powershell
.\scripts\run-docker-parity-smoke.ps1
```

## Docker compose parity E2E

Runs a compose-based Gateway parity stack:

- `gateway` mock (WebSocket control plane),
- `rust-agent` (this Rust defender runtime),
- `producer` (mock inbound action event),
- `assertor` (verifies emitted `security.decision` contract).

```bash
bash ./scripts/run-docker-compose-parity.sh
```

```powershell
.\scripts\run-docker-compose-parity.ps1
```

Restart/reconnect chaos variant (restarts `rust-agent` during multi-event replay):

```bash
bash ./scripts/run-docker-compose-parity-chaos.sh
```

```powershell
.\scripts\run-docker-compose-parity-chaos.ps1
```
