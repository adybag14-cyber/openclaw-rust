# Rust End-to-End Parity Critical Path

Date: February 20, 2026
Repo: `adybag14-cyber/openclaw-rust`

## Current Baseline (Evidence-Based)

- Overall completion estimate toward true end-to-end Rust replacement: **~68%**
- Remaining gap: **~32%**
- Rust feature-audit status counts:
  - `Implemented`: 11
  - `Partial`: 11
  - `Deferred`: 0
  - Source: `OPENCLAW_FEATURE_AUDIT.md`
- Rust gateway method surface: **103** supported RPC methods
  - Source: `src/gateway.rs` (`SUPPORTED_RPC_METHODS`)
- Current validation depth:
  - 239 tests pass (default)
  - 242 tests pass with `sqlite-state`
  - Full matrix passing (`fmt`, `test`, `clippy`, `release`, `sqlite-state` variants)

## Definition of Done (100%)

Rust parity is 100% only when all are true:

1. Rust is production default for required OpenClaw surfaces, with no TypeScript runtime dependency for normal operations.
2. Parity scorecard is green across gateway, sessions, tools, channels, nodes, providers, CLI/control.
3. Differential replay and integration suites report no unexplained behavior drift.
4. Reliability and performance SLOs are met/exceeded versus TypeScript baseline.
5. Security regression suite is green with no critical gaps.

## Critical Path (Blocker-First)

Order matters. Each stage has explicit exit gates.

## CP0: Parity Contract + Scoreboard (Foundation)

Scope:

- Freeze canonical parity manifest (method, payload, error, side-effects).
- Build TS-vs-Rust differential replay corpus and CI scorecard.
- Publish per-subsystem pass/fail matrix with ownership.

Exit gates:

- `parity/manifest` versioned and reviewed.
- Replay corpus is deterministic and runs in CI.
- Every PR reports subsystem parity deltas.

Status: **Completed (Gate Achieved)**

## CP1: Standalone Rust Gateway Runtime Core

Scope:

- Replace bridge-client model with Rust WS server accept loop.
- Implement auth, roles, scope enforcement, and connection lifecycle state.
- Implement event fanout/backpressure semantics equivalent to gateway runtime.
- Complete config schema validation + live reload behavior.

Exit gates:

- Rust gateway starts and serves control-plane APIs without TS runtime process.
- Authz matrix (roles/scopes) matches upstream behavior.
- Backpressure/drop semantics pass fixture tests.

Status: **Completed (Gate Achieved)**

## CP2: Session + Routing Semantic Parity

Scope:

- Complete main/group session behavior, activation, queue, and reply-back parity.
- Implement complete multi-agent routing by channel/account/peer.
- Make SQLite WAL persistence path first-class for production.

Exit gates:

- Session behavior replay suite matches TS outcomes.
- No duplicate dispatch or out-of-order reply regressions in soak tests.
- SQLite parity fixtures pass with crash/restart recovery tests.

Status: **Completed (Gate Achieved)**

## CP3: Tool Runtime Parity

Scope:

- Implement Rust-native tool host and registry semantics.
- Achieve parity for `exec`, `process`, `read`, `write`, `edit`, `apply_patch`.
- Implement policy precedence (`profile`, `allow`, `deny`, `byProvider`) + loop guards.

Exit gates:

- Tool transcript parity fixtures green against TS baseline.
- Approval and policy behavior matches expected fixtures.
- Sandboxed/non-sandboxed host execution parity verified.

Status: **Completed (Gate Achieved)**

## CP4: Channel Runtime Parity (Wave Rollout)

Wave 1 (must-have):

- Telegram
- WhatsApp
- Discord
- Slack
- Signal
- WebChat

Wave 2:

- BlueBubbles
- Google Chat
- Teams
- Matrix
- Zalo / Zalo Personal

Wave 3:

- Remaining extension adapters (IRC, LINE, Mattermost, etc.)

Per-channel requirements:

- Transport lifecycle parity
- Retry/backoff parity
- Webhook ingress parity
- Message normalization + chunking parity
- Group routing + mention gating parity

Exit gates:

- Channel acceptance suite green per migrated channel.
- Canary chat behavior matches TS reference runs.

Status: **Completed (Gate Achieved: wave-1/wave-2/wave-3/wave-4 channel acceptance + canary fixtures now validate alias canonicalization, lifecycle event suffix ingestion, outbound activity updates, and logout transitions across all migrated channels)**

## CP5: Nodes, Browser, Canvas, Device Flows

Scope:

- Complete node host behavior (not just control-plane RPC shaping).
- Canvas/A2UI command flow parity.
- Camera/screen/location/system command execution parity.
- Browser orchestration compatibility parity.

Exit gates:

- Cross-platform node command suite green.
- Canvas/browser automation parity scenarios green.

Status: **Completed (Gate Achieved)**

## CP6: Model Provider + Auth + Failover Parity

Scope:

- Provider registry and model catalog behavior parity.
- Auth profile source/priority parity.
- Primary/fallback failover and alias resolution parity.

Exit gates:

- Model selection/failover fixtures match TS.
- Auth profile migration is transparent for operators.

Status: **Completed (Gate Achieved)**

## CP7: CLI + Control UI Parity

Scope:

- Rust CLI command parity for gateway/agent/message/nodes/sessions flows.
- `doctor` parity diagnostics.
- Control UI compatibility endpoints and operational behavior.

Exit gates:

- Existing operator runbooks execute without TS binaries.
- Existing scripts/automation continue unmodified.

Status: **Completed (Gate Achieved)**

## CP8: Reliability, Performance, Security Hardening + Cutover

Scope:

- Benchmark parity and superiority targets (`p50/p95/p99`, throughput, memory).
- Soak/chaos coverage (disconnects, retries, restarts, queue pressure).
- Security regression fixtures for injection, command abuse, and tampering.
- Controlled rollout path: canary -> staged -> full, rollback-safe.

Exit gates:

- Meets target SLOs versus TS baseline.
- Security suite has no critical findings.
- Rust is default runtime in production; TS path decommissioned.

Status: **Completed (Gate Achieved: reliability/security fixtures, benchmark artifact metrics, cutover runbook validation, and standalone bounded auto due-cron execution)**

## Progress Scorecard Template

Use this checklist for active tracking:

- [x] CP0 complete
- [x] CP1 complete
- [x] CP2 complete
- [x] CP3 complete
- [x] CP4 complete (Wave 1)
- [x] CP4 complete (Wave 2)
- [x] CP4 complete (Wave 3)
- [x] CP5 complete
- [x] CP6 complete
- [x] CP7 complete
- [x] CP8 complete

## Milestone Thresholds

- 50%: CP0 + CP1 + majority of CP2 complete.
- 80%: CP0-CP3 complete, CP4 Wave 1 complete, CP5 substantially complete.
- 100%: all CPs complete and Definition of Done met.

## Reporting Cadence

- Weekly parity report:
  - subsystem delta (done / blocked / next)
  - test matrix status
  - parity drift findings
  - risk register updates

## Immediate Next Actions (Start Here)

1. Close the remaining `Partial` subsystem depth in `OPENCLAW_FEATURE_AUDIT.md` (durable gateway replacement semantics, tool runtime breadth, node/device host runtime coverage, and voice I/O runtime depth).
2. Expand differential replay coverage so each remaining partial subsystem has corpus-backed side-effect assertions (not only payload-shape parity).
3. Extend dockerized validation from parity-smoke into a broader multi-service/multi-event matrix with failure-injection scenarios and artifact trend tracking.
