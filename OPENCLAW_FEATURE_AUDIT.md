# OpenClaw Rust Rewrite Feature Audit

Date: 2026-02-18  
Audit basis: `openclaw/openclaw` `main` docs and README in this workspace

## Scope

This audit compares upstream OpenClaw capabilities with the current Rust implementation in `rust-agent/`.

Current architecture status:

- Rust currently acts as a **Gateway-compatible defender runtime**.
- It is **not yet** a full replacement for the TypeScript Gateway/runtime/channel stack.

Status legend:

- `Implemented`: Working in current Rust code.
- `Partial`: Exists but limited scope compared to upstream.
- `Not Started`: No Rust implementation yet.
- `Deferred`: Intentionally kept in upstream Gateway for now.

## Feature Matrix

CP1 update (2026-02-19):

- Rust now supports a standalone gateway runtime mode (`gateway.runtime_mode = "standalone_server"`) with native WS accept-loop handling.
- Standalone mode enforces role/scope authorization at request-dispatch boundary (`operator`/`node`, `operator.*` scope matrix).
- Standalone mode includes bounded event fanout with slow-consumer drop/prune semantics and fixture coverage.
- Standalone mode includes config schema-validated live reload polling (`gateway.server.reload_interval_secs`).

CP2 increment (2026-02-19):

- `sessions.list` now accepts route-aware filters (`channel`, `to`, `accountId`) in addition to label/agent/spawn selectors.
- `sessions.resolve` now accepts route-aware selectors (`channel`, `to`, `accountId`) for deterministic channel/account/peer session targeting.
- SQLite session-state backend now has restart-recovery fixtures that verify counters and last-seen metadata survive store re-open and continue accumulating correctly.
- Bridge queue-pressure soak coverage now asserts no duplicate dispatch and no out-of-order replies for followup-mode session execution under bounded pending-capacity pressure.
- Added repeatable CP2 parity gate runners (`scripts/parity/run-cp2-gate.ps1`, `scripts/parity/run-cp2-gate.sh`) that execute session/routing + SQLite recovery fixtures.
- CI parity workflow now includes a dedicated CP2 gate job (`session-routing-cp2`) that runs the same fixture suite on every push/PR.
- Added fixture-driven CP2 session-routing replay corpus (`tests/parity/session-routing-corpus.json`) with bridge harness assertions for mention activation, steer semantics, and followup queue-pressure FIFO prefix behavior.
- Added multi-session soak fixture (`bridge::tests::multi_session_soak_preserves_per_session_fifo_without_duplicates`) to stress parallel session churn and assert per-session FIFO ordering with no duplicate dispatches.
- Expanded gateway payload parity corpus with multi-agent route fixtures for `sessions.list`/`sessions.resolve` that validate combined `channel + to + accountId` selector behavior and agent-scoped route resolution.
- Added bridge-level reply-back parity fixture (`bridge::tests::reply_back_payload_preserves_group_and_direct_delivery_context`) that validates `replyBack` and delivery-context equivalence for group and direct session paths.
- Added route-selector collision parity fixtures for `sessions.list`/`sessions.resolve` that disambiguate same-peer traffic across channel/account boundaries.
- Added `sessions.resolve` precedence fixtures that validate explicit `sessionKey` resolution wins over route selectors when both are present.
- Expanded CP2 parity gate scripts with dedicated gateway tests for shared-peer route disambiguation and `sessionKey` precedence.
- Added hybrid `sessions.resolve` replay fixtures covering `label + route selectors`, `sessionId + route selectors`, and partial route-selector resolution without `accountId`.
- Added deterministic partial-route collision fixtures for `sessions.resolve` (most-recent update wins, key-order tie-break when timestamps match) plus replay-corpus sleep support (`__sleep__` with `sleepMs`) for stable timing-sensitive parity checks.
- CP2 parity gate scripts now emit structured artifacts (`cp2-gate.log`, fixture duration TSV, JSON metrics, markdown summary) and CI uploads them for trend tracking/drift review.

CP3 increment (2026-02-19):

- Added Rust tool runtime policy matcher foundation with upstream-aligned precedence stages (`profile`, `allow`, `deny`, `byProvider`) including group expansion, alias normalization, wildcard matching, and `apply_patch` allow-via-`exec` compatibility.
- Integrated runtime tool policy enforcement into defender classification (`tool_policy_deny`) so denied tools are blocked consistently before execution.
- Added configurable tool-loop guard foundation (warning/critical thresholds, bounded history) and integrated it into defender decisions (`tool_loop_warning`, `tool_loop_critical`).
- Added Rust-native CP3 tool host/runtime execution path (`src/tool_runtime.rs`) for `read`, `write`, `edit`, `apply_patch`, `exec`, and `process`, including bounded in-memory transcript and background process session registry semantics.
- Wired CP3 policy precedence + loop guard directly into tool-host execution (not only defender classification), including warning/critical loop escalation behavior at runtime.
- Added transcript-driven CP3 replay corpus (`tests/parity/tool-runtime-corpus.json`) with sandboxed and non-sandboxed fixture coverage.
- Expanded CP3 gate runners (`scripts/parity/run-cp3-gate.ps1`, `scripts/parity/run-cp3-gate.sh`) and CI job (`tool-runtime-cp3`) to run runtime corpus/background process fixtures and publish CP3 artifacts (including corpus snapshot).

CP4 increment (2026-02-19):

- Expanded channel registry coverage to include Wave-1 channels `signal` and `webchat` in addition to `telegram`, `whatsapp`, `discord`, and `slack`.
- Added channel-runtime parity helper foundations for chat-type normalization (`dm -> direct`), mention-gating decision semantics (with control-command bypass variant), outbound text chunking modes (`length`/`newline`), and deterministic retry/backoff scheduling.
- Scheduler mention-activation now uses channel mention-gating semantics that avoid false skips when mention detection is unavailable.
- Bridge reconnect loop now uses deterministic retry/backoff helper policy instead of ad-hoc inline growth logic.
- Added repeatable CP4 gate runners (`scripts/parity/run-cp4-gate.ps1`, `scripts/parity/run-cp4-gate.sh`) plus CI gate job (`channel-runtime-cp4`) with artifact publishing (`parity/generated/cp4/*`).
- Added event-driven channel runtime registry for Wave-1 transport lifecycle parity: channel status/runtime events now hydrate `channels.status` snapshots with per-account `running`, `connected`, `reconnectAttempts`, `lastError`, and activity timestamps.
- Added runtime lifecycle updates for outbound sends/polls and `channels.logout` so account snapshots reflect outbound activity and logout stop transitions.
- Expanded runtime ingestion compatibility for upstream-shaped `channelAccounts` snapshots and added `channelSystemImages` + `channelMeta.systemImage` parity fields; `channels.logout` now reports `supported/loggedOut/cleared` using runtime activity state instead of fixed placeholders.
- Added webhook-ingress runtime parity for `.message` events that only carry channel hints in payload (no channel token in event name), and outbound activity parity for `chat.send`/`chat.inject` on `webchat`.
- Added runtime default-account hint ingestion (`channelDefaultAccountId` and per-event `defaultAccountId`) so `channels.status` channel summaries follow upstream default-account selection semantics when multiple accounts are present.
- Enforced strict `channels.status`/`channels.logout` request-shape parity by rejecting unknown params (`deny_unknown_fields`) to match upstream schema validation behavior.
- Added channel-alias canonicalization parity for CP4 (`tg`, `wa`, `signal-cli`, `web-chat`) so webhook ingress/runtime hydration and `channels.logout` operate on upstream-equivalent canonical channel ids.
- Added channel-summary probe metadata parity (`channels.*.lastProbeAt` null-or-timestamp + optional `channels.*.probe`) and logout payload parity field `envToken` for `channels.logout`.
- Added alias- and snake_case-compatible runtime map ingestion for `channelAccounts`/`channelDefaultAccountId` payloads so upstream transport snapshots keyed by aliases (for example `wa`, `signal-cli`) hydrate canonical Rust channel status views.
- Added nested `channels.<id>.defaultAccountId`/`default_account_id` runtime ingestion parity so default-account hints embedded inside per-channel runtime payloads drive `channels.status` default-account and summary selection.
- Aligned `channels.logout` runtime semantics with upstream "no active session" behavior by avoiding synthetic runtime-account creation when the requested account is absent (`cleared/loggedOut` remain `false`).
- Aligned synthetic `channels.status` defaults for unset runtime snapshots to upstream-friendly values (`configured=false`, `linked=false`) while keeping `enabled=true`.
- Added account-level probe timestamp parity in `channels.status` (`channelAccounts.*.lastProbeAt` null-or-timestamp) with runtime probe payload pass-through when `probe=false` and rust probe payload override when `probe=true`.
- Expanded channel runtime metadata ingestion/output parity for account snapshots (`dmPolicy`, `allowFrom`, token-source fields, `baseUrl`, `allowUnmentionedGroups`, `cliPath`, `dbPath`, `port`, plus `probe`/`audit`/`application` payload objects).
- Added CP4 account-identity parity hardening: case-insensitive logout matching, canonical default-account ID casing based on known runtime accounts, default-account-first ordering in `channelAccounts`, account name/display-name ingestion, string-list parsing for `allowFrom`, and numeric default-account hint ingestion across channel runtime payload shapes.

CP5 increment (2026-02-19):

- `browser.request` now supports runtime node-proxy orchestration when a browser-capable paired node is available (`caps: ["browser"]` or `commands: ["browser.proxy"]`), with deterministic target resolution and upstream-aligned command-allowlist enforcement.
- Browser proxy invokes now use waitable node runtime completion via `node.invoke.result`, including bounded timeout cancellation semantics and explicit unavailable/error shaping when proxy results are missing/failed.
- Added `browser.open` parity routing via gateway dispatcher (`/tabs/open` proxy shape with optional profile + node target passthrough).
- Added `canvas.present` parity routing via waitable node-runtime invoke flow (`node.invoke.request` + timeout cancellation + payload/error shaping).
- Added explicit CP5 node command-suite fixture for declared `camera.snap`, `screen.record`, `location.get`, and `system.run` command paths via `node.invoke`.
- Added repeatable CP5 parity gate runners (`scripts/parity/run-cp5-gate.ps1`, `scripts/parity/run-cp5-gate.sh`) and CI gate job (`node-browser-canvas-cp5`) with artifact publishing (`parity/generated/cp5/*`).

CP6 increment (2026-02-19):

- Added CP6 model/provider parity foundation for provider alias normalization in session model overrides (`z.ai`/`z-ai` -> `zai`, `qwen` -> `qwen-portal`, `opencode-zen` -> `opencode`, `kimi-code` -> `kimi-coding`).
- Added OpenAI Codex provider routing normalization (`openai/gpt-5.3-codex*` -> `openai-codex/*`) and Anthropic shorthand model alias normalization (`sonnet-4.5` -> `claude-sonnet-4-5`, etc.).
- Added provider failover-chain helper foundation surfaced in model catalog payloads (`fallbackProviders`) with regression fixtures.
- Added session auth-profile override parity fields (`authProfileOverride`, `authProfileOverrideSource`, `authProfileOverrideCompactionCount`) and model-patch clearing semantics to avoid stale profile pinning when model/provider changes.
- Added runtime model failover execution semantics with provider-attempt traces under auth-profile cooldown pressure, including fallback selection in `agent`, `session.status`, `sessions.patch`, and `sessions.compact` responses.
- Added compaction-driven auto profile rotation for auto-selected auth profiles with bounded, deterministic provider profile ordering.

CP7 increment (2026-02-19):

- Added Rust CLI `doctor` command starter with deterministic non-interactive diagnostics and optional JSON output (`doctor --non-interactive --json`).
- Added CP7 parity gate runners (`scripts/parity/run-cp7-gate.ps1`, `scripts/parity/run-cp7-gate.sh`) and CI gate job (`cli-control-cp7`) with artifact publishing (`parity/generated/cp7/*`).
- Added CP7 fixture coverage for CLI doctor parsing/report behavior plus control-plane update sentinel doctor-hint contract checks.

- `Runtime portability`: Upstream OpenClaw feature surface is macOS/Linux/Windows workflow and Linux service deployment. Rust status is `Implemented`. Notes: Rust toolchain pinned to 1.83; Ubuntu build script and systemd user unit included.
- `Gateway protocol connectivity`: Upstream OpenClaw feature surface is WS control plane (`connect`, events, session/gateway methods). Rust status is `Partial`. Notes: Rust bridge uses typed frame helpers (`req`/`resp`/`event`), method-family classification, known-method registry, `connect` post-handshake rejection parity ("connect is only valid as the first request"), and RPC dispatcher coverage for gateway introspection (`health`, `status`), usage summaries (`usage.status`, `usage.cost`), system control parity (`last-heartbeat`, `set-heartbeats`, `system-presence`, `system-event`, `wake`), talk/channel control parity (`talk.config`, `talk.mode`, `channels.status`, `channels.logout`), TTS/VoiceWake control parity (`tts.status`, `tts.enable`, `tts.disable`, `tts.convert`, `tts.setProvider`, `tts.providers`, `voicewake.get`, `voicewake.set` with in-memory provider/enable/trigger state + conversion payload shaping), web login parity (`web.login.start`, `web.login.wait` with in-memory QR session lifecycle), browser parity (`browser.request` validation + no-node unavailable contract + browser-node proxy runtime path via `node.invoke.result` completion), exec approvals parity (`exec.approvals.get`, `exec.approvals.set`, `exec.approvals.node.get`, `exec.approvals.node.set` with base-hash concurrency checks + socket token redaction + bounded per-node snapshots), exec approval workflow parity (`exec.approval.request`, `exec.approval.waitDecision`, `exec.approval.resolve` with bounded pending map + timeout/grace cleanup + two-phase acceptance path), chat RPC parity (`chat.history`, `chat.send`, `chat.abort`, `chat.inject` with bounded in-memory run registry, idempotent run-status responses, session-level abort semantics, assistant injection path, inbound send sanitization/null-byte rejection, stop-command abort routing, transcript-backed history payload shaping, `id`/`parentId` chat history chain fields, and `chat.inject` final event payload emission with upstream-aligned `seq = 0`), fixture-driven payload parity corpus checks (`dispatcher_payload_corpus_matches_upstream_fixtures` against `tests/parity/gateway-payload-corpus.json`, currently covering `chat.*`, `tts.*`, `voicewake.*`, `web.login.*`, `update.run`, `sessions.*`, `browser.request`, `config.*`, `logs.tail`, `cron.*`, `exec.approvals.*`, `exec.approval.*`, and `wizard.*`), outbound send parity (`send` with idempotency replay cache, internal `webchat` channel rejection guidance, channel validation/defaulting, and mirrored session transcript writes), poll parity (`poll` with idempotency replay cache, channel poll-capability gating, and Telegram-only option guards for `durationSeconds`/`isAnonymous`), update parity (`update.run` with restart-sentinel shaped payload), wizard parity (`wizard.start`, `wizard.next`, `wizard.cancel`, `wizard.status` with single-running-session guard), device pairing/token parity (`device.pair.list`, `device.pair.approve`, `device.pair.reject`, `device.pair.remove`, `device.token.rotate`, `device.token.revoke` with bounded in-memory pending/paired registry + token summaries/redaction), node pairing parity (`node.pair.request`, `node.pair.list`, `node.pair.approve`, `node.pair.reject`, `node.pair.verify`, `node.rename`, `node.list`, `node.describe`, `node.invoke`, `node.invoke.result`, `node.event` with bounded in-memory pending/paired registry + token verification + paired-node inventory views + invoke/result runtime queue), model/agent control parity (`models.list`, `agents.list`, `agents.create`, `agents.update`, `agents.delete`, `agents.files.list`, `agents.files.get`, `agents.files.set`, `agent`, `agent.identity.get`, `agent.wait` with idempotent started/in_flight/ok run lifecycle + wait integration + slash reset handling for `/new` and `/reset`), skills control parity (`skills.status`, `skills.bins`, `skills.install`, `skills.update` with API-key normalization + in-memory config state), cron RPC parity (`cron.list`, `cron.status`, `cron.add`, `cron.update`, `cron.remove`, `cron.run`, `cron.runs` with bounded in-memory run logs), config/log parity (`config.get`, `config.set`, `config.patch`, `config.apply`, `config.schema`, `logs.tail`), plus session control methods (`sessions.list`, `sessions.preview`, `sessions.patch`, `sessions.resolve`, `sessions.reset`, `sessions.delete`, `sessions.compact`, `sessions.usage`, `sessions.usage.timeseries`, `sessions.usage.logs`, `sessions.history`, `sessions.send`, `session.status`) including `sessions.send` rejection of internal-only `webchat` with actionable `chat.send` guidance; full RPC dispatch parity still pending.
- `Full Gateway replacement`: Upstream OpenClaw feature surface is sessions, presence, routing, config mutations, cron/webhooks, and control UI serving. Rust status is `Partial`. Notes: Rust now covers a broad in-memory gateway RPC surface (including cron CRUD/run/runs/status), but TS still owns durable cron scheduling, webhook transport side effects, and UI serving/runtime orchestration.
- `Session model`: Upstream OpenClaw feature surface is `main` session, group isolation, activation/queue policies, and reply-back. Rust status is `Partial`. Notes: First-pass per-session scheduler now supports `followup`/`steer`/`collect` queue modes plus group activation gating (`mention`/`always`), with state counters + bounded in-memory session transcript (`sessions.history`/`sessions.send`) + session usage aggregation (`sessions.usage`, date-range inputs, context-weight placeholder, and extended envelope fields for totals/actions/aggregates) + filtered listing (`includeGlobal`, `includeUnknown`, `agentId`, `search`, `label`, `spawnedBy`) + optional list hint fields (`displayName`, `derivedTitle`, `lastMessagePreview`, `lastAccountId`, `deliveryContext`, `totalTokensFresh`) + metadata-aware session resolution (`label`, `spawnedBy`) + `sessions.history` lookup parity via `key` aliases and `sessionId` + `sessions.preview` output-key parity for requested aliases + explicit per-session `sessionId` tracking (including `sessions.resolve` by `sessionId` and `sessions.reset` ID rotation) + canonical alias/short-key normalization for session RPC lookups and mutations + reset/compact parameter/default parity (`reason` = `new|reset`, `maxLines >= 1`, default compact window 400) + extended `sessions.patch` parity (`key`, `ok/path/key/entry`, tuning fields, canonical value normalization, explicit `null` clears, `reasoningLevel/responseUsage` `"off"` clear semantics, `sendPolicy` constrained to `allow|deny|null`, label uniqueness, consistent label length constraints (max 64) across patch/list/resolve without silent truncation, subagent-only immutable `spawnedBy`/`spawnDepth`) + `sessions.delete`/`sessions.compact` envelope parity (`path`, `archived`) including `deleteTranscript` handling + last-decision persistence (JSON default, optional SQLite WAL); advanced routing/reply-back parity still pending.
- `Channel integrations`: Upstream OpenClaw feature surface is WhatsApp, Telegram, Discord, Slack, IRC, Signal, Google Chat, Teams, Matrix, etc. Rust status is `Partial`. Notes: Rust adapter scaffold now includes `telegram`, `whatsapp`, `discord`, `slack`, `signal`, `webchat`, and generic extraction, plus wave-1 runtime helpers for normalization, mention gating, chunking, retry/backoff, and event-driven channel runtime snapshot ingestion; full transport/webhook/runtime parity remains pending.
- `Tool execution layer`: Upstream OpenClaw feature surface is `exec`, `process`, `apply_patch`, browser/canvas/nodes, message, gateway, and sessions\_\* methods. Rust status is `Partial`. Notes: CP3 core host parity for `exec/process/read/write/edit/apply_patch` is now implemented with transcript/runtime fixtures; broader browser/canvas/nodes/message/gateway tool-family parity remains pending.
- `Nodes + device features`: Upstream OpenClaw feature surface is macOS/iOS/Android nodes, camera/screen/location/system.run, and canvas A2UI. Rust status is `Partial`. Notes: Rust now has bounded in-memory node/device pairing + invoke/event runtime semantics, browser proxy orchestration, canvas present command routing, and explicit CP5 fixture coverage for declared camera/screen/location/system command invoke paths; dedicated node host processes and full platform transport implementations remain pending.
- `Voice stack`: Upstream OpenClaw feature surface is Voice Wake, Talk Mode, and audio I/O flows. Rust status is `Partial`. Notes: Talk mode, `tts.*`, and VoiceWake control-plane methods (`voicewake.get`, `voicewake.set`) are available in-memory; full audio I/O runtime flows remain out of current Rust scope.
- `Model/provider layer`: Upstream OpenClaw feature surface is provider catalog, auth profiles, and failover/routing. Rust status is `Implemented`. Notes: Rust now includes CP6 parity for provider/model alias normalization, session auth-profile override lifecycle semantics, runtime provider failover execution under profile-cooldown pressure, and failover-chain shaping in model catalog metadata.
- `CLI + control surface`: Upstream OpenClaw feature surface is operator CLI command parity, `doctor` diagnostics, and control UI compatibility pathways. Rust status is `Partial`. Notes: CP7 starter adds Rust `doctor` diagnostics command and CI gate coverage; broader command/runbook parity and full control UI/runtime serving remain pending.
- `Prompt-injection defense`: Upstream OpenClaw feature surface is prompt pattern detection plus exfiltration/bypass heuristics. Rust status is `Implemented`. Notes: `prompt_guard.rs` with pattern scoring and heuristic boosts.
- `Command safety defense`: Upstream OpenClaw feature surface is blocked regex patterns plus allow-prefix policy and escalation/pipe checks. Rust status is `Implemented`. Notes: `command_guard.rs` with risk scoring model.
- `Host integrity defense`: Upstream OpenClaw feature surface is baseline hashing and tamper detection on protected paths. Rust status is `Implemented`. Notes: `host_guard.rs` checks hash drift/missing files.
- `VirusTotal integration`: Upstream OpenClaw feature surface is external URL/file reputation signal. Rust status is `Implemented`. Notes: `virustotal.rs` supports URL/file hash lookup and risk mapping.
- `Decision policy engine`: Upstream OpenClaw feature surface is risk aggregation to `allow`/`review`/`block` with thresholds. Rust status is `Implemented`. Notes: `security/mod.rs` classifier with `audit_only` override.
- `Tool/channel policy controls`: Upstream OpenClaw feature surface is per-tool policy floors and channel-aware risk weighting. Rust status is `Implemented`. Notes: `tool_policies`, `tool_risk_bonus`, and `channel_risk_bonus` are configurable in TOML, and can now be overridden via signed startup policy bundles.
- `Idempotency dedupe`: Upstream OpenClaw feature surface is repeated action/request suppression. Rust status is `Partial`. Notes: Request id/signature idempotency cache added with TTL + bounded entries.
- `Channel driver abstraction`: Upstream OpenClaw feature surface is channel-specific frame parsing adapters. Rust status is `Partial`. Notes: Trait-based registry added with `whatsapp`, `telegram`, `slack`, `discord`, and generic drivers.
- `Quarantine records`: Upstream OpenClaw feature surface is persisting blocked action payloads for forensics. Rust status is `Implemented`. Notes: Append-only JSON files in configured quarantine directory.
- `Backpressure + memory controls`: Upstream OpenClaw feature surface is bounded worker concurrency, queue cap, eval timeout, and memory metrics. Rust status is `Implemented`. Notes: Semaphore + queue bounds + timeout + Linux RSS sampler.
- `Test coverage (Rust)`: Upstream OpenClaw feature surface is unit/integration validation for core safety/runtime behavior. Rust status is `Partial`. Notes: Core security/bridge/channel adapters/replay harness covered, including bridge-level mention-activation and steer-queue semantics; full end-to-end Gateway/channel matrix still pending.
- `Dockerized validation`: Upstream OpenClaw feature surface is containerized CI-style runtime test matrix. Rust status is `Partial`. Notes: Added Docker parity smoke harness (`deploy/Dockerfile.parity`, run scripts) for default + `sqlite-state`, plus compose-based Gateway parity stack (`deploy/docker-compose.parity.yml`) with mock gateway + producer + assertor around the Rust runtime; broader real-channel/container matrix remains pending.

## Custom Defender Goal Coverage

### Goal 1: Ubuntu 20.04 Rust runtime

- `Implemented` for build/deploy baseline:
  - `scripts/build-ubuntu20.sh`
  - `deploy/openclaw-agent-rs.service`

### Goal 2: Faster and more RAM-efficient behavior

- `Implemented` in phase-1 runtime controls:
  - bounded worker pool
  - bounded queue
  - per-eval timeout
  - low-overhead Linux RSS sampling

- `Partial` for deeper optimizations:
  - pooled binary event buffers
  - scheduler/session hot-path tuning and indexing on SQLite backend for larger deployments
  - throughput benchmarking vs upstream runtime

### Goal 3: Defender AI + VirusTotal hardening against prompt injection and host compromise

- `Implemented`:
  - prompt-injection scoring
  - command risk scoring
  - host file-integrity checks
  - VirusTotal URL/file signal fusion
  - audit-only rollout mode
  - quarantine artifacts

- `Partial`:
  - no kernel/EDR process telemetry ingestion
  - no remote attestation of runtime binary yet

## Immediate Next Build Targets

1. Expand session model parity to include group isolation, activation policy tuning, and reply-back semantics.
2. Complete CP4 wave-1 transport lifecycle + webhook ingress parity on top of the new channel-runtime helper layer.
3. Expand compose parity stack to include multi-event scenarios (retry/backoff and reconnect assertions).
4. Add signed policy bundle rotation/distribution workflow (key rotation + staged rollout).
