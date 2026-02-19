#!/usr/bin/env bash
set -euo pipefail

CARGO_BIN="${CARGO_BIN:-cargo}"
TOOLCHAIN="${TOOLCHAIN:-}"
PARITY_ARTIFACT_DIR="${PARITY_ARTIFACT_DIR:-parity/generated/cp8}"

mkdir -p "${PARITY_ARTIFACT_DIR}"

log_file="${PARITY_ARTIFACT_DIR}/cp8-gate.log"
results_file="${PARITY_ARTIFACT_DIR}/cp8-fixture-results.tsv"
summary_file="${PARITY_ARTIFACT_DIR}/cp8-gate-summary.md"
metrics_file="${PARITY_ARTIFACT_DIR}/cp8-metrics.json"

: > "${log_file}"
echo -e "test\tduration_ms\tstatus" > "${results_file}"

tests=(
  "bridge::tests::replay_harness_with_real_defender"
  "bridge::tests::multi_session_soak_preserves_per_session_fifo_without_duplicates"
  "security::prompt_guard::tests::scores_prompt_injection_patterns"
  "security::command_guard::tests::blocks_known_destructive_patterns"
  "security::tests::tool_loop_detection_escalates_warning_then_critical"
  "security::policy_bundle::tests::loads_valid_signed_bundle_and_applies_policy_patch"
)

total_duration_ms=0
passed=0
reliability_fixtures=0
security_fixtures=0

run_fixture() {
  local test_name="$1"
  local start_ms
  start_ms="$(date +%s%3N)"
  echo "[parity] running CP8 fixture: ${test_name}" | tee -a "${log_file}"

  local cmd=("${CARGO_BIN}")
  if [[ -n "${TOOLCHAIN}" ]]; then
    cmd+=("+${TOOLCHAIN}")
  fi
  cmd+=(test "${test_name}" -- --nocapture)

  if ! "${cmd[@]}" 2>&1 | tee -a "${log_file}"; then
    local end_ms
    end_ms="$(date +%s%3N)"
    local duration_ms=$((end_ms - start_ms))
    echo -e "${test_name}\t${duration_ms}\tfail" >> "${results_file}"
    echo "[parity] CP8 fixture failed: ${test_name}" | tee -a "${log_file}"
    exit 1
  fi

  local end_ms
  end_ms="$(date +%s%3N)"
  local duration_ms=$((end_ms - start_ms))
  total_duration_ms=$((total_duration_ms + duration_ms))
  passed=$((passed + 1))
  if [[ "${test_name}" == bridge::tests::* ]]; then
    reliability_fixtures=$((reliability_fixtures + 1))
  elif [[ "${test_name}" == security::* ]]; then
    security_fixtures=$((security_fixtures + 1))
  fi
  echo -e "${test_name}\t${duration_ms}\tpass" >> "${results_file}"
}

for test_name in "${tests[@]}"; do
  run_fixture "${test_name}"
done

total_fixtures="${#tests[@]}"
if [[ "${total_fixtures}" -gt 0 ]]; then
  avg_duration_ms=$((total_duration_ms / total_fixtures))
else
  avg_duration_ms=0
fi

cat > "${summary_file}" <<EOF
## CP8 Reliability + Security Hardening Starter Gate

- Fixtures passed: ${passed}/${total_fixtures}
- Reliability fixtures: ${reliability_fixtures}
- Security fixtures: ${security_fixtures}
- Total duration: ${total_duration_ms} ms
- Avg fixture duration: ${avg_duration_ms} ms
- Artifact log: cp8-gate.log
- Artifact metrics: cp8-metrics.json
EOF

cat > "${metrics_file}" <<EOF
{
  "gate": "cp8",
  "passed": ${passed},
  "totalFixtures": ${total_fixtures},
  "totalDurationMs": ${total_duration_ms},
  "avgFixtureDurationMs": ${avg_duration_ms},
  "reliabilityFixtureCount": ${reliability_fixtures},
  "securityFixtureCount": ${security_fixtures},
  "resultsTsv": "cp8-fixture-results.tsv"
}
EOF

echo "[parity] CP8 gate passed" | tee -a "${log_file}"
