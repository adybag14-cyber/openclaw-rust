# OpenClaw Agent (Rust)

This directory contains the Rust rewrite foundation for the OpenClaw runtime.

Minimum supported Rust version: `1.83`.

What is implemented now:

- Native Rust runtime suitable for Ubuntu 20.04 deployment.
- Gateway compatibility bridge over OpenClaw's WebSocket protocol.
- Standalone Rust Gateway WebSocket runtime mode (no TypeScript gateway process).
- Defender pipeline that can block/review suspicious actions before execution.
- VirusTotal lookups (file hash + URL) to add external threat intelligence.
- Host integrity baseline checks for key runtime files.
- Bounded concurrency and queue limits to reduce memory spikes.
- Session FIFO scheduling + decision state tracking + idempotency cache.
- Typed session-key parsing (`main`, `direct`, `group`, `channel`, `cron`, `hook`, `node`).
- Typed protocol frame foundation (`req`/`resp`/`event` classification).
- Gateway RPC parity scaffold for `sessions.list`, `sessions.preview`, `sessions.patch`, `sessions.resolve`, `sessions.reset`, `sessions.delete`, `sessions.compact`, `sessions.usage`, `sessions.usage.timeseries`, `sessions.usage.logs`, `sessions.history`, `sessions.send`, and `session.status`.
- Channel adapter scaffold (`telegram`, `whatsapp`, `discord`, `slack`, `signal`, `webchat`, generic) with wave-1 channel-runtime helpers (chat-type normalization, mention gating, chunking, retry/backoff) and event-driven runtime snapshot ingestion for `channels.status`.

This is intentionally phase 1: it keeps feature coverage by integrating with the
existing Gateway protocol while replacing high-risk runtime and guardrail logic
with Rust.

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
- `security.tool_runtime_policy.profile`: base tool profile (`minimal`, `coding`, `messaging`, `full`).
- `security.tool_runtime_policy.allow` / `security.tool_runtime_policy.deny`: wildcard/group policy filters (`group:fs`, `group:runtime`, etc.).
- `security.tool_runtime_policy.byProvider`: provider/model-specific policy overrides.
- `security.tool_runtime_policy.loop_detection`: loop guard controls (`enabled`, `history_size`, `warning_threshold`, `critical_threshold`).
- `security.policy_bundle_path`: optional signed JSON policy bundle file to load at startup.
- `security.policy_bundle_key`: HMAC key used to verify the bundle signature.
- `gateway.password`: optional shared-secret password for gateway auth.
- `gateway.runtime_mode`: `bridge_client` or `standalone_server`.
- `gateway.server.bind`: standalone server bind address.
- `gateway.server.auth_mode`: `auto`, `none`, `token`, or `password`.
- `gateway.server.handshake_timeout_ms`: max connect handshake duration.
- `gateway.server.event_queue_capacity`: per-connection outbound event queue cap.
- `gateway.server.reload_interval_secs`: config live-reload polling interval (`0` disables live reload).

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

CP4 channel-runtime wave-1 foundation gate (registry + normalization + mention/chunk/retry parity helpers):

```powershell
.\scripts\parity\run-cp4-gate.ps1
```

Current payload corpus coverage: `chat.*`, `tts.*`, `voicewake.*`, `web.login.*`, `update.run`, `sessions.*` envelope/alias flows, `browser.request`, `config.*`, `logs.tail`, `cron.*`, `exec.approvals.*`, `exec.approval.*`, and `wizard.*`.

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
