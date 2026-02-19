#!/usr/bin/env bash
set -euo pipefail

artifact_dir="${PARITY_ARTIFACT_DIR:-parity/generated/cp6}"
mkdir -p "${artifact_dir}"

log_file="${artifact_dir}/cp6-gate.log"
results_file="${artifact_dir}/cp6-fixture-results.tsv"
summary_file="${artifact_dir}/cp6-gate-summary.md"
metrics_file="${artifact_dir}/cp6-metrics.json"

tests=(
  "gateway::tests::dispatcher_models_list_returns_catalog_and_rejects_unknown_params"
  "gateway::tests::dispatcher_patch_model_normalizes_provider_aliases_and_failover_provider_rules"
  "gateway::tests::model_provider_failover_chain_normalizes_aliases"
  "security::tool_policy::tests::provider_specific_rule_is_applied_after_global_policy"
  "security::tool_policy::tests::provider_model_specific_rule_beats_provider_fallback"
)

echo -e "test\tduration_ms\tstatus" > "${results_file}"
: > "${log_file}"

now_ms() {
  local ms
  ms="$(date +%s%3N 2>/dev/null || true)"
  if [[ -n "${ms}" ]]; then
    echo "${ms}"
    return
  fi
  echo "$(( $(date +%s) * 1000 ))"
}

passed=0
total_duration_ms=0

for test_name in "${tests[@]}"; do
  start_ms="$(now_ms)"
  echo "[parity] running CP6 fixture: ${test_name}" | tee -a "${log_file}"
  if cargo test "${test_name}" -- --nocapture 2>&1 | tee -a "${log_file}"; then
    end_ms="$(now_ms)"
    duration_ms="$(( end_ms - start_ms ))"
    total_duration_ms="$(( total_duration_ms + duration_ms ))"
    echo -e "${test_name}\t${duration_ms}\tpass" >> "${results_file}"
    passed="$(( passed + 1 ))"
  else
    end_ms="$(now_ms)"
    duration_ms="$(( end_ms - start_ms ))"
    echo -e "${test_name}\t${duration_ms}\tfail" >> "${results_file}"
    echo "[parity] CP6 fixture failed: ${test_name}" | tee -a "${log_file}"
    exit 1
  fi
done

total_fixtures="${#tests[@]}"
avg_duration_ms=0
if [[ ${total_fixtures} -gt 0 ]]; then
  avg_duration_ms="$(( total_duration_ms / total_fixtures ))"
fi

cat > "${metrics_file}" <<EOF
{
  "gate": "cp6",
  "passed": ${passed},
  "totalFixtures": ${total_fixtures},
  "totalDurationMs": ${total_duration_ms},
  "avgFixtureDurationMs": ${avg_duration_ms},
  "resultsTsv": "$(basename "${results_file}")"
}
EOF

cat > "${summary_file}" <<EOF
## CP6 Model Provider/Auth/Failover Foundation Gate

- Fixtures passed: ${passed}/${total_fixtures}
- Total duration: ${total_duration_ms} ms
- Avg fixture duration: ${avg_duration_ms} ms
- Artifact log: $(basename "${log_file}")
- Artifact metrics: $(basename "${metrics_file}")
EOF

echo "[parity] CP6 gate passed" | tee -a "${log_file}"
