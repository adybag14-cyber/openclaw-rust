# Rust Rewrite Plan (Completed + Post-Parity Optimization)

## Goal alignment

1. Ubuntu 20.04 production runtime in Rust.
2. Better memory behavior and predictable throughput.
3. Defender layer against prompt injection, unsafe commands, and tampered host/runtime state.

## Status snapshot (2026-02-21)

1. End-to-end parity status: **complete** for required OpenClaw surfaces.
2. Audit status: `22 implemented`, `0 partial`, `0 deferred`.
3. Validation status: full default + `sqlite-state` matrix passing.

## Phase strategy

### Phase 1 (implemented in this directory)

- Rust runtime process + Gateway WebSocket compatibility.
- Typed protocol frame foundation (`req`/`resp`/`event`) and method-family classification.
- Gateway known-method registry plus first RPC dispatcher for:
  - `health`
  - `status`
  - `usage.status`
  - `usage.cost`
  - `sessions.list`
  - `sessions.preview`
  - `sessions.patch`
  - `sessions.resolve`
  - `sessions.reset`
  - `sessions.delete`
  - `sessions.compact`
  - `sessions.usage`
  - `sessions.usage.timeseries`
  - `sessions.usage.logs`
  - `sessions.history`
  - `sessions.send`
  - `session.status`
- Extended `sessions.list` with filter parity for `includeGlobal`, `includeUnknown`, `agentId`, and `search`.
- Extended `sessions.patch` + `sessions.resolve` with metadata parity for `label` and `spawnedBy` filtered resolution.
- Extended `sessions.usage` with date-range handling (`startDate`/`endDate`) and optional context-weight output placeholder.
- Extended `sessions.usage` envelope parity with `updatedAt`, `startDate`/`endDate`, totals, actions, and aggregate sections (`messages`, `tools`, `byAgent`, `byChannel`, `daily`).
- Extended `sessions.list` + `sessions.patch` parity with upstream-style fields:
  - `sessions.list` now supports `label`/`spawnedBy` filters and optional `includeDerivedTitles`/`includeLastMessage` hint fields.
  - `sessions.patch` now accepts `key` in addition to `sessionKey` and returns a parity-style envelope (`ok`, `path`, `key`, `entry`).
- Extended `sessions.patch` with upstream-style session tuning fields and clear semantics:
  - Added `thinkingLevel`, `verboseLevel`, `reasoningLevel`, `responseUsage`, `elevatedLevel`, `execHost`, `execSecurity`, `execAsk`, `execNode`, `model`, and `spawnDepth`.
  - Explicit `null` values now clear prior overrides for patchable session fields.
  - Added parity guardrails for patch mutations: unique labels plus subagent-only immutable `spawnedBy`/`spawnDepth`.
  - Added canonical normalization/validation for tuning knobs (thinking/reasoning/verbose/elevated/exec).
- Extended `sessions.delete` + `sessions.compact` response parity with upstream-style `path` and `archived` envelope fields.
- Added `sessions.delete` handling for `deleteTranscript` to suppress transcript-archive hints when requested.
- Added explicit `sessionId` tracking on session entries, `sessions.resolve` lookup by `sessionId`, and `sessions.reset` session-id rotation.
- Added session-key normalization to canonicalize aliases/short forms (`main`, channel-scoped keys) across session RPC operations.
- Tightened `sessions.reset`/`sessions.compact` input parity (`reason` limited to `new|reset`, `maxLines >= 1`, compact default window 400).
- Tightened `sessions.patch.sendPolicy` parity to upstream schema (`allow|deny|null` only).
- Added `sessions.list` delivery-context parity hints (`lastAccountId`, `deliveryContext`) and `totalTokensFresh` compatibility fields.
- Added `sessions.history` parity lookups for both `key` aliases and `sessionId`.
- Aligned patch-clear parity for `reasoningLevel`/`responseUsage` so explicit `"off"` clears persisted overrides.
- Aligned preview response parity to preserve requested keys in `sessions.preview` output.
- Tightened session label validation parity (`label` max length 64; no silent truncation on patch inputs).
- Enforced matching label validation for `sessions.list`/`sessions.resolve` query filters.
- Rust defender policy engine with bounded worker concurrency.
- Prompt injection scoring + command risk scoring.
- Host integrity baseline checks.
- VirusTotal signal integration for URL/file indicators.
- Quarantine ledger for blocked actions.

### Phase 2 (completed)

- Move session scheduler and idempotency dedupe cache to Rust.
- Implemented first-pass session FIFO scheduler with configurable queue modes:
  - `followup`: preserve all follow-ups in order.
  - `steer`: keep only the latest pending follow-up while a session is active.
  - `collect`: merge prompt-only follow-ups into a single pending turn.
- Added group activation gating (`mention` or `always`) before scheduling group-context actions.
- Added typed session-key parsing (`main/direct/group/channel/cron/hook/node`) for routing-aware scheduler behavior.
- Implemented first pass idempotency dedupe cache with TTL + bounded entries.
- Implemented dual-backend session state tracking:
  - JSON (default)
  - SQLite WAL backend behind `sqlite-state` feature (auto-selected for `.db/.sqlite/.sqlite3` paths)
- Introduce a compact internal event model (`bytes` + pooled buffers).
- Advanced routing parity (group isolation/activation policies/reply-back) completed.

### Phase 3 (completed)

- Migrate core channel adapters incrementally behind trait drivers.
- Added trait-based channel adapter scaffold (`whatsapp`, `telegram`, `slack`, `discord`, generic fallback) with capability descriptors.
- Keep protocol schema stable for existing clients (macOS/iOS/Android/Web/CLI).

### Phase 4 (completed)

- Decommission TypeScript runtime path after parity tests pass.

### Phase 5 (post-parity roadmap)

- Throughput trend automation (upstream-vs-Rust benchmarks in CI).
- Memory hot-path tuning (event fanout pooling and queue pressure profiling).
- Expanded real-transport and failure-injection integration coverage.

## Performance design choices

- Bounded concurrent evaluations via semaphore.
- Bounded queue target in config.
- Lightweight Linux RSS sampler for runtime memory observability.
- Timeout for each security evaluation to prevent backlog growth.
- Optional external Intel (VirusTotal) behind short timeout.
- Quarantine writes are append-only JSON files for low contention and post-incident forensics.

## Security design choices

- Risk-based decision model (`allow`, `review`, `block`).
- Pattern and behavior based prompt-injection detection.
- Command policy with explicit deny patterns and allow-prefix policy.
- Runtime file hash checks to detect tampering.
- Audit-only mode for safe rollout before hard block enforcement.
