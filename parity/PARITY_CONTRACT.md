# Rust Parity Contract (Phase 1)

This contract defines the current machine-checked RPC method surface parity check.

## Source Of Truth

- Rust method surface: `src/gateway.rs` -> `SUPPORTED_RPC_METHODS`
- Upstream base surface: `../openclaw/src/gateway/server-methods-list.ts` -> `BASE_METHODS`
- Upstream handler surface: `../openclaw/src/gateway/server-methods/*.ts` -> exported `*Handlers` maps

## Contract Rule

- Every method present in upstream `BASE_METHODS` is expected to exist in Rust `SUPPORTED_RPC_METHODS` unless explicitly documented as deferred.
- The method diff artifacts are generated and committed from scripts in `scripts/parity/`.

## Commands

From repo root:

```powershell
.\scripts\parity\method-surface-diff.ps1 -Surface both
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

## PR Gate

- For any gateway parity change, regenerate method diff artifacts and include them in the commit.
