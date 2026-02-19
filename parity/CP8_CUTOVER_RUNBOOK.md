# CP8 Cutover Runbook

Date: 2026-02-19

This runbook defines the rollout and rollback process for Rust runtime hardening
cutover once parity gates are green.

## Preconditions

- CP0 through CP8 parity gates are green in CI for the release candidate commit.
- `cargo test`, `clippy -D warnings`, and release builds pass for default and
  `sqlite-state`.
- Parity artifacts for `cp2` through `cp8` are generated and committed.
- Defender policy bundle signature verification is enabled for startup policy
  updates.

## Canary

- Deploy Rust runtime to a single canary host with production-like traffic.
- Keep TypeScript runtime available in standby mode without active routing.
- Run canary soak for at least 24 hours:
  - No crash-looping process restarts.
  - No sustained queue saturation alerts.
  - No critical defender regressions (`prompt_guard`, `command_guard`,
    `tool_loop`, `policy_bundle`).
- Capture canary metrics snapshot:
  - RPC latency percentiles (`p50`, `p95`, `p99`)
  - Throughput (ops/sec)
  - RSS memory trend

## Staged

- Expand to a staged subset of hosts/channels after canary sign-off.
- Keep automated parity replay and CP8 hardening gate results attached to the
  rollout change ticket.
- Validate channel/session behavior and defender events against expected
  baselines for each stage increment.
- Block promotion if any stage shows:
  - elevated error-rate trend,
  - sustained latency regression over canary baseline,
  - unapproved command execution policy drift.

## Full Cutover

- Promote Rust runtime as default for all targeted production hosts.
- Disable TypeScript runtime routing paths for normal operation.
- Keep TypeScript binaries available only for emergency rollback window.
- Verify post-cutover health:
  - gateway status/health commands succeed on all nodes,
  - parity gate artifacts remain green for the cutover commit,
  - no unresolved critical security findings.

## Rollback

- Trigger rollback immediately if any critical condition occurs:
  - sustained runtime crash loops,
  - critical security regression,
  - severe latency/throughput regression impacting SLA.
- Rollback steps:
  1. Repoint traffic to the previous known-good runtime release.
  2. Restore previous policy bundle and runtime config snapshot.
  3. Confirm gateway health and channel delivery recovery.
  4. Open incident record with parity artifact links and mitigation status.
- Keep forensic artifacts (`cp8` logs/metrics, security events, runtime logs)
  attached to the incident for root-cause follow-up.
