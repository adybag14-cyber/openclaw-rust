#!/usr/bin/env bash
set -euo pipefail

CARGO_BIN="${CARGO_BIN:-cargo}"
TOOLCHAIN="${TOOLCHAIN:-}"
PARITY_ARTIFACT_DIR="${PARITY_ARTIFACT_DIR:-parity/generated/cp7}"

mkdir -p "${PARITY_ARTIFACT_DIR}"

log_file="${PARITY_ARTIFACT_DIR}/cp7-gate.log"
results_file="${PARITY_ARTIFACT_DIR}/cp7-fixture-results.tsv"
summary_file="${PARITY_ARTIFACT_DIR}/cp7-gate-summary.md"
metrics_file="${PARITY_ARTIFACT_DIR}/cp7-metrics.json"

: > "${log_file}"
echo -e "test\tduration_ms\tstatus" > "${results_file}"

tests=(
  "tests::cli_parses_doctor_command_and_flags"
  "tests::doctor_report_marks_config_load_failure_as_blocking"
  "tests::doctor_report_warns_when_docker_is_unavailable"
  "gateway::tests::dispatcher_update_and_web_login_methods_report_expected_payloads"
)

total_duration_ms=0
passed=0

run_fixture() {
  local test_name="$1"
  local start_ms
  start_ms="$(date +%s%3N)"
  echo "[parity] running CP7 fixture: ${test_name}" | tee -a "${log_file}"

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
    echo "[parity] CP7 fixture failed: ${test_name}" | tee -a "${log_file}"
    exit 1
  fi

  local end_ms
  end_ms="$(date +%s%3N)"
  local duration_ms=$((end_ms - start_ms))
  total_duration_ms=$((total_duration_ms + duration_ms))
  passed=$((passed + 1))
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
## CP7 CLI + Control UI Starter Gate

- Fixtures passed: ${passed}/${total_fixtures}
- Total duration: ${total_duration_ms} ms
- Avg fixture duration: ${avg_duration_ms} ms
- Artifact log: cp7-gate.log
- Artifact metrics: cp7-metrics.json
EOF

cat > "${metrics_file}" <<EOF
{
  "gate": "cp7",
  "passed": ${passed},
  "totalFixtures": ${total_fixtures},
  "totalDurationMs": ${total_duration_ms},
  "avgFixtureDurationMs": ${avg_duration_ms},
  "resultsTsv": "cp7-fixture-results.tsv"
}
EOF

echo "[parity] CP7 gate passed" | tee -a "${log_file}"
