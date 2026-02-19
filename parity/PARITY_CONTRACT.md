# Rust Parity Contract (Phase 1/2 + CP2/CP3/CP4/CP5/CP6/CP7/CP8 Gates)

This contract defines the machine-checked parity checks that currently gate Rust
gateway parity work.

## Source Of Truth

- Rust method surface: `src/gateway.rs` -> `SUPPORTED_RPC_METHODS`
- Upstream base surface: `../openclaw/src/gateway/server-methods-list.ts` -> `BASE_METHODS`
- Upstream handler surface: `../openclaw/src/gateway/server-methods/*.ts` -> exported `*Handlers` maps
- Versioned manifest: `parity/manifest/PARITY_MANIFEST.v1.json`

## Phase 1: Method Surface Rule

- Every method present in upstream `BASE_METHODS` is expected to exist in Rust `SUPPORTED_RPC_METHODS` unless explicitly documented as deferred.
- The method diff artifacts are generated and committed from scripts in `scripts/parity/`.

## Phase 2: Payload Shape Rule

- Curated request/response/event fixtures are stored in `tests/parity/gateway-payload-corpus.json`.
- Rust gateway test `dispatcher_payload_corpus_matches_upstream_fixtures` replays the corpus and checks JSON-pointer payload shape expectations.
- Payload fixtures are anchored to upstream `../openclaw/src/gateway/server-methods/*.ts` handler behavior.
- Current corpus coverage includes `chat.*`, `tts.*`, `voicewake.*`, `web.login.*`, `update.run`, `sessions.*` envelope/alias behavior, `browser.request` unavailable contract, `config.*`, `logs.tail`, `cron.*`, `exec.approvals.*`, `exec.approval.*`, and `wizard.*` lifecycle error/shape checks.

## Commands

From repo root:

```powershell
.\scripts\parity\method-surface-diff.ps1 -Surface both
```

```powershell
.\scripts\parity\payload-shape-diff.ps1
```

```powershell
.\scripts\parity\build-scoreboard.ps1
```

```powershell
.\scripts\parity\run-replay-corpus.ps1
```

```powershell
.\scripts\parity\run-cp1-gate.ps1
```

```powershell
.\scripts\parity\run-cp2-gate.ps1
```

```powershell
.\scripts\parity\run-cp3-gate.ps1
```

```powershell
.\scripts\parity\run-cp4-gate.ps1
```

```powershell
.\scripts\parity\run-cp5-gate.ps1
```

```powershell
.\scripts\parity\run-cp6-gate.ps1
```

```powershell
.\scripts\parity\run-cp7-gate.ps1
```

```powershell
.\scripts\parity\run-cp8-gate.ps1
```

Optional upstream location override:

```powershell
.\scripts\parity\method-surface-diff.ps1 -UpstreamRepoPath "C:\path\to\openclaw"
```

## Generated Artifacts

- `parity/generated/upstream-methods.base.json`
- `parity/generated/upstream-methods.handlers.json`
- `parity/generated/rust-methods.json`
- `parity/generated/method-surface-diff.json`
- `parity/method-surface-report.md`
- `tests/parity/gateway-payload-corpus.json` (fixture corpus)
- `parity/generated/cp2/*` (CP2 gate fixture logs/metrics/summary)
- `tests/parity/tool-runtime-corpus.json` (CP3 transcript/runtime fixture corpus)
- `parity/generated/cp3/*` (CP3 gate logs/metrics/summary + runtime corpus artifact)
- `parity/generated/cp4/*` (CP4 channel-runtime gate logs/metrics/summary)
  - includes wave-1 channel lifecycle/runtime snapshot fixtures (`channels.status` event-ingest parity + logout transition checks)
- `parity/generated/cp5/*` (CP5 node/browser/canvas/device gate logs/metrics/summary)
- `parity/generated/cp6/*` (CP6 model provider/auth/failover gate logs/metrics/summary)
- `parity/generated/cp7/*` (CP7 CLI/control parity gate logs/metrics/summary)
- `parity/generated/cp8/*` (CP8 reliability/security starter gate logs/metrics/summary)

## PR Gate

- For any gateway method-surface parity change, regenerate method diff artifacts and include them in the commit.
- For any gateway behavior-parity change touching payload shape, update `tests/parity/gateway-payload-corpus.json` and keep `dispatcher_payload_corpus_matches_upstream_fixtures` passing.
- PR CI must publish `parity/generated/parity-scoreboard.md` as job summary, including subsystem status deltas versus `parity/manifest/scoreboard-baseline.json`.
- PR CI must keep CP1 standalone runtime fixtures green (`run-cp1-gate.sh`) for authz matrix and event backpressure/drop semantics.
- PR CI must publish/upload CP2 gate artifacts (`parity/generated/cp2`) for session/routing trend tracking.
- PR CI must keep CP3 parity fixtures green (`run-cp3-gate.sh`) and publish/upload `parity/generated/cp3` artifacts.
- PR CI must keep CP4 channel-runtime fixtures green (`run-cp4-gate.sh`) and publish/upload `parity/generated/cp4` artifacts.
- PR CI must keep CP5 node/browser/canvas/device fixtures green (`run-cp5-gate.sh`) and publish/upload `parity/generated/cp5` artifacts.
- PR CI must keep CP6 model/auth/failover fixtures green (`run-cp6-gate.sh`) and publish/upload `parity/generated/cp6` artifacts.
- PR CI must keep CP7 CLI/control parity fixtures green (`run-cp7-gate.sh`) and publish/upload `parity/generated/cp7` artifacts.
- PR CI must keep CP8 reliability/security starter fixtures green (`run-cp8-gate.sh`) and publish/upload `parity/generated/cp8` artifacts.
