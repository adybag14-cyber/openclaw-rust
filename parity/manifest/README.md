# Parity Manifest

This folder contains versioned parity-contract artifacts used to gate Rust parity work.

## Active Manifest

- `PARITY_MANIFEST.v1.json`

## Baseline Artifact

- `scoreboard-baseline.json` (generated from current parity snapshot)

## Review Process

For any contract change:

1. Update manifest JSON.
2. Re-run CP0 tooling:
   - `.\scripts\parity\method-surface-diff.ps1 -Surface both -UpstreamRepoPath ..\openclaw`
   - `.\scripts\parity\build-scoreboard.ps1`
   - `.\scripts\parity\run-replay-corpus.ps1`
   - `.\scripts\parity\run-cp1-gate.ps1`
3. Include regenerated parity artifacts in the same PR.
4. Add or update issue progress note for CP0/CP1 tracker.

## Review Log

- 2026-02-19: Manifest v1 created and adopted for CP0 gate tracking.
- 2026-02-19: Manifest v1 extended for CP1 standalone gateway gate tracking.
