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
- Added CP4 wave-2 channel registry foundations for `bluebubbles`, `googlechat`, `msteams`, `matrix`, `zalo`, and `zalouser`, including upstream alias canonicalization (`google-chat`, `gchat`, `teams`, `bb`, `zl`, `zlu`) and channel catalog/status exposure.
- Added send parity alias canonicalization for wave-2 channels so `send`/`poll` channel routing accepts upstream-style alias ids and emits canonical channel ids in responses.
- Added CP4 wave-3 core channel registry foundations for `irc` and `imessage`, including upstream alias canonicalization (`internet-relay-chat`, `imsg`), channel catalog label/system-image parity, and runtime payload alias ingestion in `channels.status`.
- Added CP4 wave-3 extension tranche foundations for `feishu`, `mattermost`, `line`, `nextcloud-talk`, `nostr`, and `tlon`, including alias canonicalization (`lark`, `nc-talk`, `nc`, `urbit`), channel catalog label/system-image parity, and runtime payload alias ingestion in `channels.status`.
- Added CP4 transport lifecycle event parity for lightweight channel events (`*.connected`, `*.reconnecting`, `*.error`, `*.disconnected`) so runtime account state updates correctly even when events do not include full runtime maps.
- Added CP4 cross-wave acceptance/canary fixture coverage (`gateway::tests::dispatcher_channel_acceptance_canary_covers_wave_channels`) validating alias canonicalization + runtime lifecycle + outbound activity + logout semantics across all wave channels.
- Added durable channel-runtime parity: runtime now supports config-driven channel runtime store paths (`channels.runtimeStorePath`/`runtime_store_path`, `channelRuntime.storePath`/`store_path`, and `runtime.channelRuntimeStorePath`) with disk-backed channel/account lifecycle snapshot persistence and restart recovery across dispatcher instances.
- Added CP4 channel activity-suffix parity for `*.sent`/`*.outbound`/`*.delivery` and `*.received`/`*.incoming` event shapes (including `message_sent`/`message-received` forms), plus nested channel/account alias extraction under `meta/context/ctx/runtime/data`, so webhook ingress updates `lastOutboundAt`/`lastInboundAt` consistently beyond plain `*.message` events.

CP4 increment (2026-02-21):

- Expanded replay-corpus side-effect assertions for outbound channel parity by adding fixture coverage for `send` and `poll` transcript mirroring (`sessions.history` source/context fields), plus explicit webchat-send and unsupported-poll-channel guardrail contracts in `tests/parity/gateway-payload-corpus.json`.
- Re-ran full CP gate matrix (`run-cp0-gate` through `run-cp9-gate`) and published refreshed parity artifacts under `parity/generated/*`.

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

- Added runbook-compatible Rust CLI command groups for `gateway`, `agent`, `message send`, `nodes`, and `sessions` in addition to `doctor`, aligned to upstream operator command families.
- Added deterministic non-interactive `doctor` diagnostics with JSON output (`doctor --non-interactive --json`) and retained control-plane update sentinel `doctorHint` contract checks.
- Expanded CP7 parity gate runners (`scripts/parity/run-cp7-gate.ps1`, `scripts/parity/run-cp7-gate.sh`) with command parse + RPC execution fixtures and kept CI gate job (`cli-control-cp7`) artifact publishing (`parity/generated/cp7/*`).

CP8 increment (2026-02-19):

- Added CP8 hardening gate expansion with reliability chaos fixtures (`bridge` queue-pressure soak, scheduler drop semantics, standalone gateway slow-consumer drop semantics, and retry/backoff policy checks).
- Added CP8 benchmark fixture (`gateway::tests::dispatcher_status_benchmark_emits_latency_profile`) emitting latency percentiles (`p50/p95/p99`), throughput, and RSS metrics to `parity/generated/cp8/cp8-benchmark.json`.
- Added CP8 cutover runbook (`parity/CP8_CUTOVER_RUNBOOK.md`) and gate-time validation for required rollout sections (`Canary`, `Staged`, `Full Cutover`, `Rollback`).
- Added CP8 CI gate job (`hardening-cp8`) publishing full hardening artifacts (`parity/generated/cp8/*`), including benchmark metrics.
- Added standalone auto-cron durability parity: bounded due-run worker now executes due jobs without explicit `cron.run` calls and applies the same cron side-effects (`system-presence` update + logs) as manual runs.
- Added cron webhook delivery parity hardening: cron callbacks now mirror upstream `finished` event payload shape, require non-empty summaries, attach optional `Authorization: Bearer <cron.webhookToken>` headers, support legacy `notify: true` fallback via global `cron.webhook` config, and emit one-time deprecation warnings when legacy fallback is used.
- Added durable send/poll idempotency parity: runtime now supports config-driven (`idempotency.sendStorePath`, TTL, max-entries) disk-backed idempotency cache persistence with restart replay and expired-entry pruning semantics.
- Added durable session-registry parity: runtime now supports config-driven session store paths (`session.storePath`/`store_path` and `session.statePath`/`state_path`) with disk-backed snapshot persistence for session entries/history/usage metadata and restart recovery across dispatcher instances.
- Added durable device-pair registry parity: runtime now supports config-driven device pair store paths (`devicePair.storePath`/`store_path`, `device.pair.storePath`/`store_path`, and `runtime.devicePairStorePath`) with disk-backed pending/paired/token snapshot persistence and restart recovery across dispatcher instances.
- Added durable node-pair registry parity: runtime now supports config-driven node pair store paths (`nodePair.storePath`/`store_path`, `node.pair.storePath`/`store_path`, and `runtime.nodePairStorePath`) with disk-backed pending/paired snapshot persistence and restart recovery across dispatcher instances.
- Hardened parity PowerShell gate runners (CP0-CP8 + replay corpus) against native stderr false-fail behavior while preserving explicit exit-code checks, and validated CP8 shell-gate benchmark summary generation on the current scripts.

CP9 increment (2026-02-20):

- Added repeatable CP9 Docker parity gate runners (`scripts/parity/run-cp9-gate.sh`, `scripts/parity/run-cp9-gate.ps1`) that execute daemon health, Dockerfile smoke, and compose parity checks with duration/metrics artifacts.
- Added CI parity gate job (`docker-parity-cp9`) to run CP9 on Ubuntu runners and publish CP9 artifacts (`parity/generated/cp9/*`) + markdown summary into the workflow summary.
- Expanded CP9 with restart/reconnect chaos validation (`deploy/docker-compose.parity-chaos.yml`, `scripts/run-docker-compose-parity-chaos.{sh,ps1}`) and gate enforcement (`docker-compose-chaos-restart`) so containerized parity now validates decision continuity across in-run Rust agent restarts.

CP10 increment (2026-02-20):

- Added signed policy-bundle key-rotation parity with declared `keyId` support and keyring verification fallback (`security.policy_bundle_keys` + `OPENCLAW_RS_POLICY_BUNDLE_KEYS`), including strict unknown-`keyId` rejection and rotation fallback tests.
- Added repeatable policy-bundle staged rollout tooling (`scripts/security/rotate-policy-bundle.py`, `scripts/security/rotate-policy-bundle.sh`, `scripts/security/rotate-policy-bundle.ps1`) that emits canary/staged/rollback signed bundle artifacts plus a rotation manifest.
- Expanded CP3 tool-host breadth with native `gateway`, `sessions`, and `message` tool families (status/method introspection, bounded in-memory session message history/list/reset flows, and message-send alias behavior) with added CP3 gate fixture coverage.
- Expanded CP3 message-tool parity depth with explicit `message` action handling (`send`, `poll`, `react`, `reactions`, `read`, `edit`, `delete`, `pin`, `unpin`, `pins`, `permissions`, `thread-create`, `thread-list`, `thread-reply`, `member-info`, `role-info`, `channel-info`, `channel-list`, `voice-status`, `event-list`, `event-create`, `role-add`, `role-remove`, `timeout`, `kick`, `ban`) plus bounded in-memory reaction/edit/delete/pin/thread/event/member-role state, corpus fixtures, and dedicated runtime regression coverage.
- Expanded voice runtime parity depth in `tts.convert` by emitting deterministic synthesized audio payload metadata (`audioBase64`, `audioBytes`, `durationMs`, `sampleRateHz`, `channels`, `textChars`) alongside existing path/provider/output fields.
- Expanded CP9 compose validation to multi-event decision matrix coverage (allow/review/block in one run) with duplicate-decision guard assertions and scenario-driven producer/assertor fixtures.
- Added durable control-surface registry parity for `config`, `web.login`, and `wizard` runtime state via config-driven store paths with disk-backed snapshot persistence and restart recovery fixtures.
- Added local node-host runtime execution parity for declared node command flows (`browser.proxy`, `canvas.present`, `camera.snap`, `screen.record`, `location.get`, `system.run`) so eligible local nodes can complete invokes in-process without external `node.invoke.result` dependency.
- Expanded standalone gateway authorization parity so control-UI orchestration write methods (`browser.open`, `canvas.present`, `web.login.*`, `wizard.*`, and `config.*`) are explicitly covered by scope-gated authz fixtures.

CP11 increment (2026-02-20):

- Expanded CP3 tool-host execution parity to include native `browser`, `canvas`, and `nodes` tool families with runtime action coverage (`browser.open/request`, `canvas.present`, `nodes.invoke/list/status`) and guarded `system.run` execution semantics.
- Expanded CP3 replay corpus for tool runtime (`tests/parity/tool-runtime-corpus.json`) with browser/canvas/nodes fixtures and added runtime-family regression coverage in `tool_runtime` tests.
- Added channel-driver extraction parity hardening for nested transport metadata (`payload/meta/context/ctx/runtime/data` channel hints), including alias canonicalization in driver resolution paths.
- Added standalone gateway HTTP control surface parity (opt-in `gateway.server.http_bind`) with control-UI page + JSON endpoints (`/health`, `/status`, `/rpc/methods`) and end-to-end gateway-server fixture coverage.
- Expanded TTS runtime parity depth with provider-backed synthesis attempts (`openai`, `elevenlabs` via API keys) plus deterministic fallback, surfacing `providerUsed` and `synthSource` metadata for runtime observability.
- Added session-delivery context parity hardening: decision ingestion now captures `deliveryContext` hints (`channel`, `to`, `accountId`) into session registry metadata so route selectors and list views remain aligned even when events are not sourced through `sessions.send`.
- Added bridge routing parity hardening for inbound frames without explicit `sessionKey`: bridge now resolves session keys from delivery-context hints via dispatcher-side route resolution before scheduling.

CP12 increment (2026-02-20):

- Added standalone gateway runtime event-surface parity for control-plane hello payloads: advertised events now include the upstream gateway event set plus the configured decision event.
- Added configurable standalone tick cadence (`gateway.server.tick_interval_ms`, env `OPENCLAW_RS_GATEWAY_TICK_INTERVAL_MS`) with runtime `tick` event emission and hello `policy.tickIntervalMs` parity.
- Added standalone shutdown event broadcast semantics (`shutdown` with reason/timestamp payload) on graceful server termination.
- Expanded standalone control HTTP surface to support JSON RPC passthrough (`POST /rpc`) in addition to discovery/status endpoints, enabling method invocation without WS client wiring.
- Hardened control HTTP request handling with bounded header/body parsing and content-length aware reads to avoid partial-frame parsing flake conditions.
- Added parity fixtures for standalone hello event advertisement + tick emission and control HTTP RPC passthrough behavior.

CP13 increment (2026-02-20):

- Completed session reply-back edge parity for `sessions.send`: requests now accept `sessionKey|key|sessionId`, and reply-back sends can resolve target sessions via route selectors (`channel`/`to`/`accountId`/`threadId`) when explicit session keys are omitted.
- Added session parity fixtures covering route-selector reply-back resolution and `sessionId` alias behavior in `sessions.send`.
- Expanded standalone channel webhook parity with route aliases (`/webhooks/*`, `/channel/*/webhook`, singular `/channels/*/account/*/webhook`) and batched ingress (`events[]`, top-level arrays, `type/data` envelopes) wired through the same scheduler/decision pipeline.
- Added gateway-server parity fixtures for webhook route aliases and batched webhook decision dispatch.
- Expanded local node-host external runtime parity with per-command external command overrides (`nodeHost.externalCommands`) so specific node commands can delegate to dedicated host runtimes while retaining global fallback command support.
- Added CP5 parity fixture for command-specific local node-host external runtime routing.
- Expanded voice runtime depth with device-aware control paths: `talk.mode` now accepts `inputDevice`/`outputDevice`, `tts.convert` accepts `outputDevice`, and runtime payloads expose playback output-device metadata.
- Expanded voice parity fixtures to validate input/output device tracking across `talk.mode`, `tts.convert`, and `tts.status`.
- Expanded CP1 and CP5 gate definitions to include the new webhook and node-host runtime parity fixtures.

CP14 increment (2026-02-20):

- Added persistent local node-host external runtime orchestration (`nodeHost.externalPersistent`) with bounded per-runtime request queues and idle session lifecycle management.
- Added config parity fields for persistent host execution controls (`externalPersistent`, `externalQueueCapacity`, `externalIdleTimeoutMs`) with runtime aliases under `runtime.*`.
- Added external-runtime session reuse for sequential `node.invoke` calls so command-specific host processes can stay warm instead of respawning per request.
- Added CP5 node-host parity fixtures for override-only external command maps (no global fallback command required) and persistent external host session reuse semantics.
- Expanded local node-host in-process command parity with `system.which` and `system.notify` support plus fixture coverage in node invoke command suites.
- Hardened standalone control-HTTP parity fixture transport with bounded retry in test helpers to avoid intermittent `missing HTTP body` flakes on busy CI workers.

CP15 increment (2026-02-20):

- Expanded local node-host in-process command parity with `camera.clip` support (`durationMs`/`seconds`, `includeAudio`/`noAudio`, `facing`, `deviceId`, `format` payload shaping) in addition to existing `camera.snap`.
- Hardened local node-host `system.run` parity depth by accepting argv-shaped `params.command` arrays, enforcing `rawCommand` consistency checks, honoring per-request timeout aliases, supporting bounded env overrides while intentionally ignoring `PATH` overrides, and surfacing structured run metadata (`rawCommand`, `argv`, env-ignore list, timeout, `needsScreenRecording`) in result payloads.
- Expanded local node-host `system.notify` parity payload depth with `priority` + `delivery` fields (`passive|active|timeSensitive` / `system|overlay|auto`) alongside title/body/level.
- Expanded CP3 tool-runtime nodes command breadth to include `camera.clip`, `system.which`, and `system.notify`, and updated CP3 gate runners to enforce runtime-family fixture coverage.

CP16 increment (2026-02-21):

- Expanded CP3 message transport parity with channel-capability enforcement for `message` actions that depend on native adapter support (`poll`, `edit`, `delete`, `react/reactions`, and thread actions).
- Added message-channel parity fixtures validating alias normalization (`tg -> telegram`) and deterministic unsupported-channel rejections for unsupported/unknown transport capabilities.
- Hardened message tool channel resolution to derive capability checks from explicit `channel` args first and session-key channel descriptors as fallback.

CP17 increment (2026-02-21):

- Expanded local node-host/runtime parity with read-only cross-platform command stubs aligned to upstream allowlist surfaces: `camera.list`, `device.info`, `device.status`, `contacts.search`, `calendar.events`, `reminders.list`, `photos.latest`, `motion.activity`, and `motion.pedometer`.
- Expanded Rust tool-runtime `nodes` command family to expose/invoke the same read-only node command set, including updated `status`/`list` capability payloads.
- Added gateway/tool-runtime fixture coverage for the new node command tranche (declared-command invoke loops, local host runtime payload assertions, and CP3 corpus additions).

CP18 increment (2026-02-21):

- Expanded local node-host/runtime parity for canvas node command family beyond `canvas.present`: added `canvas.hide`, `canvas.navigate`, `canvas.eval`, `canvas.snapshot`, `canvas.a2ui.push`, `canvas.a2ui.pushJSONL`, and `canvas.a2ui.reset`.
- Expanded Rust tool-runtime `nodes.invoke` support for the same canvas command family with deterministic payload shaping and alias-compatible command normalization for `pushJSONL`.
- Added gateway/tool-runtime regression coverage for canvas node commands (`node.invoke` declared-command loops, local host runtime assertions, and CP3 corpus snapshots).

CP19 increment (2026-02-21):

- Expanded Rust tool-runtime `nodes` action parity beyond `status/list/invoke` by adding `describe`, `pending`, `approve`, `reject`, `notify`, and `run` action handling with upstream-shaped request keys (`node|nodeId`, `requestId`, `title/body`, command arrays).
- Hardened tool-runtime `nodes.run` parity for argv-style command payloads by adding array parsing, shell-wrapper command inference (`cmd/sh/pwsh` wrappers), and `rawCommand` consistency validation.
- Expanded CP3 nodes parity coverage with new tool-runtime regression assertions and corpus fixtures for `nodes.describe`, `nodes.notify`, and `nodes.run` array-mode execution paths.

CP20 increment (2026-02-21):

- Expanded Rust tool-runtime `message.permissions` parity to be channel-capability-aware rather than static-all-true output.
- Permissions now inherit resolved channel support for `poll`, `edit`, `delete`, `react/reactions`, and thread actions (`threadCreate/threadList/threadReply`) while preserving permissive defaults when no channel context is resolved.
- Added CP3 message-parity fixtures and regression assertions validating capability-shaped permission payloads for `slack` and `telegram`.

CP21 increment (2026-02-21):

- Expanded channel runtime activity-event parity beyond inbound/outbound markers by adding mutation suffix tracking for reactions, edits, deletes, and thread operations.
- `channels.status` account payloads now surface `lastReactionAt`, `lastEditAt`, `lastDeleteAt`, and `lastThreadAt` when mutation events are observed or ingested from runtime maps.
- Expanded CP4 parity coverage/gate fixtures to validate mutation suffix hydration (`reaction-added`, `edited`, `deleted`, `thread-reply`) and artifact emission.

CP22 increment (2026-02-21):

- Hardened channel activity classification parity for dotted multi-segment event names by adding compact token matching fallback (`*.reaction.added`, `*.thread.reply`, etc.) in addition to terminal suffix matching.
- Expanded mutation activity fixtures to include both hyphenated and dotted event shapes so runtime timestamps remain robust across upstream event-emitter variants.

CP23 increment (2026-02-21):

- Aligned tool-runtime `message` advanced-action channel matrix to upstream CLI contracts, including `read`, `pin/unpin/pins`, `thread-*`, `member/role/channel/voice/event` actions, moderation actions (`timeout`/`kick`/`ban`), and reaction/search restrictions.
- Added native `message.search` action with bounded in-memory transcript filtering (`query`, `limit`, optional `threadId`/`includeDeleted`) and channel-aware enforcement.
- Expanded `message.permissions` parity to emit full per-action booleans (including `search`, moderation, and admin-style actions) for resolved channels instead of only the previous poll/edit/delete/react/thread subset.
- Expanded CP3 corpus and runtime regression fixtures for Discord search success plus unsupported Slack search / Telegram role-add channel enforcement.

CP24 increment (2026-02-21):

- Added tool-runtime message parity actions for emoji/sticker workflows: `emoji-list`, `emoji-upload`, `sticker-send`, and `sticker-upload`.
- Added channel-aware enforcement for these actions (Discord-only upload/send flows; Slack+Discord list coverage) with bounded in-memory registries for uploaded emojis/stickers and deterministic payload shaping.
- Expanded `message.permissions` parity to include `emojiList`, `emojiUpload`, `stickerSend`, and `stickerUpload` per-channel booleans.
- Expanded CP3 corpus and regression fixtures to cover emoji/sticker success paths and unsupported-channel guardrails.

CP25 increment (2026-02-21):

- Added tool-runtime `message.broadcast` parity with bounded target fanout (`target` or `targets[]`) and optional channel fanout (`channel: all`) across registered channels.
- Added deterministic guardrails for broadcast parity: explicit unknown-channel rejection and required-target validation (`broadcast requires at least one target`).
- Expanded `message.permissions` parity to include `broadcast` capability output and aligned CP3 corpus/runtime regression fixtures for broadcast success + failure paths.

CP26 increment (2026-02-21):

- Expanded `message.send` parity to support media-first payloads (`message` or `media`), explicit channel validation, and send metadata shaping (`target`, `replyTo`, `dryRun`, `mediaCount`).
- Expanded `message.broadcast` parity to support media-only fanout payloads and `dryRun` delivery status shaping while retaining target/channel validation semantics.
- Added CP3 runtime/corpus fixtures covering media-only send + broadcast success paths, missing payload guardrails, and unsupported send-channel rejection.

CP27 increment (2026-02-21):

- Expanded `message.search` parity to require `guildId` and accept upstream-shaped filter aliases for channel/author selectors (`channelId`/`channelIds`, `authorId`/`authorIds`).
- Added deterministic in-memory search filtering for `channelIds` (mapped to thread ids) and `authorIds` (mapped to message roles) while preserving existing query + limit behavior.
- Expanded CP3 runtime/corpus fixtures with filtered search scenarios covering mixed-role/thread data and Discord-search guardrails.

CP28 increment (2026-02-21):

- Hardened Discord emoji parity by requiring `guildId` for `message.emoji-list` on Discord while preserving Slack no-guild behavior.
- Expanded `message.emoji-upload` parity to ingest optional role selectors (`roleIds`/`roleId`) and echo them in emoji payloads (`emoji.roleIds`).
- Expanded CP3 runtime/corpus fixtures for Discord emoji guardrails and role-scoped emoji upload payload shaping.

- `Runtime portability`: Upstream OpenClaw feature surface is macOS/Linux/Windows workflow and Linux service deployment. Rust status is `Implemented`. Notes: Rust toolchain pinned to 1.83; Ubuntu build script and systemd user unit included.
- `Gateway protocol connectivity`: Upstream OpenClaw feature surface is WS control plane (`connect`, events, session/gateway methods). Rust status is `Implemented`. Notes: Rust bridge uses typed frame helpers (`req`/`resp`/`event`), method-family classification, known-method registry, `connect` post-handshake rejection parity ("connect is only valid as the first request"), complete upstream base/handler RPC method coverage (100% coverage from `parity/method-surface-report.md`), and runtime event-surface parity in standalone hello payloads (upstream gateway events + configured decision event, configurable `tickIntervalMs`, periodic `tick`, and graceful `shutdown` event emission). Dispatcher coverage includes gateway introspection (`health`, `status`), usage summaries (`usage.status`, `usage.cost`), system control parity (`last-heartbeat`, `set-heartbeats`, `system-presence`, `system-event`, `wake`), talk/channel control parity (`talk.config`, `talk.mode`, `channels.status`, `channels.logout`), TTS/VoiceWake control parity (`tts.status`, `tts.enable`, `tts.disable`, `tts.convert`, `tts.setProvider`, `tts.providers`, `voicewake.get`, `voicewake.set` with in-memory provider/enable/trigger state + conversion payload shaping), web login parity (`web.login.start`, `web.login.wait` with in-memory QR session lifecycle), browser parity (`browser.request` validation + no-node unavailable contract + browser-node proxy runtime path via `node.invoke.result` completion), exec approvals parity (`exec.approvals.get`, `exec.approvals.set`, `exec.approvals.node.get`, `exec.approvals.node.set` with base-hash concurrency checks + socket token redaction + bounded per-node snapshots), exec approval workflow parity (`exec.approval.request`, `exec.approval.waitDecision`, `exec.approval.resolve` with bounded pending map + timeout/grace cleanup + two-phase acceptance path), chat RPC parity (`chat.history`, `chat.send`, `chat.abort`, `chat.inject` with bounded in-memory run registry, idempotent run-status responses, session-level abort semantics, assistant injection path, inbound send sanitization/null-byte rejection, stop-command abort routing, transcript-backed history payload shaping, `id`/`parentId` chat history chain fields, and `chat.inject` final event payload emission with upstream-aligned `seq = 0`), fixture-driven payload parity corpus checks (`dispatcher_payload_corpus_matches_upstream_fixtures` against `tests/parity/gateway-payload-corpus.json`, currently covering `chat.*`, `tts.*`, `voicewake.*`, `web.login.*`, `update.run`, `sessions.*`, `browser.request`, `config.*`, `logs.tail`, `cron.*`, `exec.approvals.*`, `exec.approval.*`, and `wizard.*`), outbound send parity (`send` with idempotency replay cache, internal `webchat` channel rejection guidance, channel validation/defaulting, and mirrored session transcript writes), poll parity (`poll` with idempotency replay cache, channel poll-capability gating, and Telegram-only option guards for `durationSeconds`/`isAnonymous`), update parity (`update.run` with restart-sentinel shaped payload), wizard parity (`wizard.start`, `wizard.next`, `wizard.cancel`, `wizard.status` with single-running-session guard), device pairing/token parity (`device.pair.list`, `device.pair.approve`, `device.pair.reject`, `device.pair.remove`, `device.token.rotate`, `device.token.revoke` with bounded in-memory pending/paired registry + token summaries/redaction), node pairing parity (`node.pair.request`, `node.pair.list`, `node.pair.approve`, `node.pair.reject`, `node.pair.verify`, `node.rename`, `node.list`, `node.describe`, `node.invoke`, `node.invoke.result`, `node.event` with bounded in-memory pending/paired registry + token verification + paired-node inventory views + invoke/result runtime queue), model/agent control parity (`models.list`, `agents.list`, `agents.create`, `agents.update`, `agents.delete`, `agents.files.list`, `agents.files.get`, `agents.files.set`, `agent`, `agent.identity.get`, `agent.wait` with idempotent started/in_flight/ok run lifecycle + wait integration + slash reset handling for `/new` and `/reset`), skills control parity (`skills.status`, `skills.bins`, `skills.install`, `skills.update` with API-key normalization + in-memory config state), cron RPC parity (`cron.list`, `cron.status`, `cron.add`, `cron.update`, `cron.remove`, `cron.run`, `cron.runs` with bounded in-memory run logs), config/log parity (`config.get`, `config.set`, `config.patch`, `config.apply`, `config.schema`, `logs.tail`), plus session control methods (`sessions.list`, `sessions.preview`, `sessions.patch`, `sessions.resolve`, `sessions.reset`, `sessions.delete`, `sessions.compact`, `sessions.usage`, `sessions.usage.timeseries`, `sessions.usage.logs`, `sessions.history`, `sessions.send`, `session.status`) including `sessions.send` rejection of internal-only `webchat` with actionable `chat.send` guidance.
- `Full Gateway replacement`: Upstream OpenClaw feature surface is sessions, presence, routing, config mutations, cron/webhooks, and control UI serving. Rust status is `Implemented`. Notes: Rust now covers the standalone gateway runtime end-to-end for control-plane operation (WS accept loop, auth/roles/scopes, bounded broadcast/backpressure, cron CRUD + due-run scheduling + webhook delivery semantics, session/routing surfaces, and config mutation flows) and exposes an opt-in control HTTP surface (`gateway.server.http_bind`) for UI, health/status/method discovery, and JSON RPC passthrough (`POST /rpc`) without TypeScript runtime dependency.
- `Session model`: Upstream OpenClaw feature surface is `main` session, group isolation, activation/queue policies, and reply-back. Rust status is `Implemented`. Notes: Per-session scheduler supports `followup`/`steer`/`collect` queue modes plus group activation gating (`mention`/`always`), with state counters + bounded in-memory session transcript (`sessions.history`/`sessions.send`) + session usage aggregation (`sessions.usage`, date-range inputs, context-weight placeholder, and extended envelope fields for totals/actions/aggregates) + filtered listing (`includeGlobal`, `includeUnknown`, `agentId`, `search`, `label`, `spawnedBy`) + optional list hint fields (`displayName`, `derivedTitle`, `lastMessagePreview`, `lastAccountId`, `deliveryContext`, `totalTokensFresh`) + metadata-aware session resolution (`label`, `spawnedBy`) + route-selector-backed resolution (`channel`/`to`/`accountId`/`threadId`) including bridge-side fallback resolution for inbound events without explicit `sessionKey` + `sessions.history` lookup parity via `key` aliases and `sessionId` + `sessions.preview` output-key parity for requested aliases + explicit per-session `sessionId` tracking (including `sessions.resolve` by `sessionId` and `sessions.reset` ID rotation) + canonical alias/short-key normalization for session RPC lookups and mutations + reset/compact parameter/default parity (`reason` = `new|reset`, `maxLines >= 1`, default compact window 400) + extended `sessions.patch` parity (`key`, `ok/path/key/entry`, tuning fields, canonical value normalization, explicit `null` clears, `reasoningLevel/responseUsage` `"off"` clear semantics, `sendPolicy` constrained to `allow|deny|null`, label uniqueness, consistent label length constraints (max 64) across patch/list/resolve without silent truncation, subagent-only immutable `spawnedBy`/`spawnDepth`) + `sessions.delete`/`sessions.compact` envelope parity (`path`, `archived`) including `deleteTranscript` handling + last-decision persistence (JSON default, optional SQLite WAL) + reply-back edge resolution via `sessionKey|key|sessionId` and route-selector fallback.
- `Channel integrations`: Upstream OpenClaw feature surface is WhatsApp, Telegram, Discord, Slack, IRC, Signal, Google Chat, Teams, Matrix, etc. Rust status is `Partial`. Notes: Rust adapter scaffold now includes `telegram`, `whatsapp`, `discord`, `irc`, `slack`, `signal`, `imessage`, `webchat`, `bluebubbles`, `googlechat`, `msteams`, `matrix`, `zalo`, `zalouser`, `feishu`, `mattermost`, `line`, `nextcloud-talk`, `nostr`, `tlon`, and generic extraction, plus wave-1 + wave-2 + wave-3 runtime helpers for normalization, mention gating, chunking, retry/backoff, alias canonicalization, and event-driven channel runtime snapshot ingestion plus config-driven disk-backed channel runtime persistence/restart recovery, standalone webhook route aliases, batched webhook ingress, and channel-capability-aware message-tool action enforcement (`poll`, `edit`, `delete`, `react/reactions`, thread actions); full adapter-native transport/runtime parity remains pending.
- `Tool execution layer`: Upstream OpenClaw feature surface is `exec`, `process`, `apply_patch`, browser/canvas/nodes, message, gateway, and sessions\_\* methods. Rust status is `Implemented`. Notes: CP3 host parity now covers `exec/process/read/write/edit/apply_patch` plus native `gateway`/`sessions`/`message` and `browser`/`canvas`/`nodes` tool families, including explicit `message` action parity (`send`, `broadcast`, `poll`, `react`, `reactions`, `read`, `search`, `edit`, `delete`, `pin`, `unpin`, `pins`, `permissions`, `thread-create`, `thread-list`, `thread-reply`, `member-info`, `role-info`, `channel-info`, `channel-list`, `voice-status`, `event-list`, `event-create`, `emoji-list`, `emoji-upload`, `sticker-send`, `sticker-upload`, `role-add`, `role-remove`, `timeout`, `kick`, `ban`) with bounded in-memory reaction/edit/delete/pin/thread/event/member-role/emoji/sticker state and fallback targeting, guarded `nodes.invoke` `system.run` behavior, and runtime-node command breadth for `camera.clip`, `system.which`, and `system.notify`, with corpus + gate fixtures and runtime-family regression coverage.
- `Nodes + device features`: Upstream OpenClaw feature surface is macOS/iOS/Android nodes, camera/screen/location/system.run, and canvas A2UI. Rust status is `Partial`. Notes: Rust now has node/device pairing + invoke/event runtime semantics with bounded state and config-driven disk-backed device/node pair snapshot persistence plus restart recovery, browser proxy orchestration, expanded canvas command routing (`canvas.present`, `canvas.hide`, `canvas.navigate`, `canvas.eval`, `canvas.snapshot`, `canvas.a2ui.push`, `canvas.a2ui.pushJSONL`, `canvas.a2ui.reset`), explicit CP5 fixture coverage for declared camera/screen/location/system command invoke paths (including `camera.clip`, `system.which`, and `system.notify`), expanded read-only node command parity stubs (`camera.list`, `device.info/status`, `contacts.search`, `calendar.events`, `reminders.list`, `photos.latest`, `motion.activity`, `motion.pedometer`) across gateway local runtime + tool-runtime nodes.invoke flows, richer local-host `system.run` parameter parity (`argv` + `rawCommand` consistency, timeout/env alias handling, PATH-override ignore semantics), per-command external host-runtime delegation (`nodeHost.externalCommands`), and config-driven persistent local host process managers (`nodeHost.externalPersistent`) with bounded queue + idle-lifecycle controls; full platform transport implementations remain pending.
- `Voice stack`: Upstream OpenClaw feature surface is Voice Wake, Talk Mode, and audio I/O flows. Rust status is `Partial`. Notes: Talk mode, `tts.*`, and VoiceWake control-plane methods (`voicewake.get`, `voicewake.set`) are available in-memory, `talk.mode` now supports input/output device selection, and `tts.convert` now supports output-device targeting plus provider-backed synthesis attempts (`openai`/`elevenlabs` when API keys are configured) with deterministic fallback + payload metadata (`audioBase64`, byte count, duration, sample rate, channel count, `providerUsed`, `synthSource`); full live audio capture/playback device flows remain out of current Rust scope.
- `Model/provider layer`: Upstream OpenClaw feature surface is provider catalog, auth profiles, and failover/routing. Rust status is `Implemented`. Notes: Rust now includes CP6 parity for provider/model alias normalization, session auth-profile override lifecycle semantics, runtime provider failover execution under profile-cooldown pressure, and failover-chain shaping in model catalog metadata.
- `CLI + control surface`: Upstream OpenClaw feature surface is operator CLI command parity, `doctor` diagnostics, and control UI compatibility pathways. Rust status is `Implemented`. Notes: Rust now exposes gateway/agent/message/nodes/sessions command families plus `doctor`, with CP7 gate fixtures and CI artifact enforcement, and standalone control UI serving via the gateway HTTP surface.
- `Prompt-injection defense`: Upstream OpenClaw feature surface is prompt pattern detection plus exfiltration/bypass heuristics. Rust status is `Implemented`. Notes: `prompt_guard.rs` with pattern scoring and heuristic boosts.
- `Command safety defense`: Upstream OpenClaw feature surface is blocked regex patterns plus allow-prefix policy and escalation/pipe checks. Rust status is `Implemented`. Notes: `command_guard.rs` with risk scoring model.
- `Host integrity defense`: Upstream OpenClaw feature surface is baseline hashing and tamper detection on protected paths. Rust status is `Implemented`. Notes: `host_guard.rs` checks hash drift/missing files.
- `VirusTotal integration`: Upstream OpenClaw feature surface is external URL/file reputation signal. Rust status is `Implemented`. Notes: `virustotal.rs` supports URL/file hash lookup and risk mapping.
- `Decision policy engine`: Upstream OpenClaw feature surface is risk aggregation to `allow`/`review`/`block` with thresholds. Rust status is `Implemented`. Notes: `security/mod.rs` classifier with `audit_only` override.
- `Tool/channel policy controls`: Upstream OpenClaw feature surface is per-tool policy floors and channel-aware risk weighting. Rust status is `Implemented`. Notes: `tool_policies`, `tool_risk_bonus`, and `channel_risk_bonus` are configurable in TOML, and can now be overridden via signed startup policy bundles.
- `Idempotency dedupe`: Upstream OpenClaw feature surface is repeated action/request suppression. Rust status is `Implemented`. Notes: Request id/signature decision-cache idempotency remains TTL + bounded-entry controlled, and gateway `send`/`poll` idempotency replay is now disk-backed with runtime-configurable store path, TTL expiry pruning, bounded max entries, and restart recovery fixture coverage.
- `Channel driver abstraction`: Upstream OpenClaw feature surface is channel-specific frame parsing adapters. Rust status is `Implemented`. Notes: Trait-based registry covers all parity channels (`whatsapp`, `telegram`, `slack`, `discord`, `irc`, `signal`, `imessage`, `webchat`, `bluebubbles`, `googlechat`, `msteams`, `matrix`, `zalo`, `zalouser`, `feishu`, `mattermost`, `line`, `nextcloud-talk`, `nostr`, `tlon`) with alias canonicalization and nested transport channel-hint extraction for driver routing.
- `Quarantine records`: Upstream OpenClaw feature surface is persisting blocked action payloads for forensics. Rust status is `Implemented`. Notes: Append-only JSON files in configured quarantine directory.
- `Backpressure + memory controls`: Upstream OpenClaw feature surface is bounded worker concurrency, queue cap, eval timeout, and memory metrics. Rust status is `Implemented`. Notes: Semaphore + queue bounds + timeout + Linux RSS sampler.
- `Test coverage (Rust)`: Upstream OpenClaw feature surface is unit/integration validation for core safety/runtime behavior. Rust status is `Partial`. Notes: Core security/bridge/channel adapters/replay harness are covered, including bridge-level mention-activation + steer-queue semantics, standalone gateway HTTP control surface fixtures (including webhook alias + batch-ingress coverage), tool-runtime browser/canvas/nodes runtime-family fixtures, expanded CP1/CP5 gate fixture matrices, and replay-corpus side-effect assertions for outbound `send`/`poll` transcript mirroring plus channel guardrails; broader real-transport end-to-end channel matrix remains pending.
- `Dockerized validation`: Upstream OpenClaw feature surface is containerized CI-style runtime test matrix. Rust status is `Implemented`. Notes: Added Docker parity smoke harness (`deploy/Dockerfile.parity`, run scripts) for default + `sqlite-state`, plus compose-based Gateway parity stacks (`deploy/docker-compose.parity.yml`, `deploy/docker-compose.parity-chaos.yml`) with mock gateway + producer + assertor around the Rust runtime; CP9 gate runners + CI artifact publishing now enforce daemon/smoke/compose + restart/reconnect chaos checks continuously, including multi-event allow/review/block matrix scenarios with duplicate-decision guards.

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

1. Expand adapter-native channel transport parity beyond webhook ingress/runtime state tracking (per-channel send/edit/delete/react/thread flows where supported).
2. Extend the new persistent local node-host process managers into full cross-platform node transport/runtime coverage.
3. Expand voice runtime from device-aware control-plane parity to live audio capture/playback device streams.
4. Expand Rust validation into broader real-transport channel integration matrices and replay drift assertions.
