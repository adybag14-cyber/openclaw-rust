#!/usr/bin/env bash
set -euo pipefail

artifact_dir="${PARITY_ARTIFACT_DIR:-parity/generated/cp3}"
mkdir -p "${artifact_dir}"

log_file="${artifact_dir}/cp3-gate.log"
results_file="${artifact_dir}/cp3-fixture-results.tsv"
summary_file="${artifact_dir}/cp3-gate-summary.md"
metrics_file="${artifact_dir}/cp3-metrics.json"

tests=(
  "security::tool_policy::tests::profile_coding_expands_group_runtime_and_fs"
  "security::tool_policy::tests::deny_takes_precedence_over_allow"
  "security::tool_policy::tests::provider_specific_rule_is_applied_after_global_policy"
  "security::tool_policy::tests::provider_model_specific_rule_beats_provider_fallback"
  "security::tool_policy::tests::allowlisted_exec_implies_apply_patch"
  "security::tool_loop::tests::emits_warning_and_critical_on_repeated_identical_tool_calls"
  "security::tests::tool_runtime_policy_profile_blocks_non_profile_tools"
  "security::tests::tool_loop_detection_escalates_warning_then_critical"
  "tool_runtime::tests::tool_runtime_corpus_matches_expected_outcomes"
  "tool_runtime::tests::tool_runtime_policy_and_loop_guard_enforced_on_tool_host"
  "tool_runtime::tests::tool_runtime_background_exec_process_poll_roundtrip"
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
  echo "[parity] running CP3 fixture: ${test_name}" | tee -a "${log_file}"
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
    echo "[parity] CP3 fixture failed: ${test_name}" | tee -a "${log_file}"
    exit 1
  fi
done

total_fixtures="${#tests[@]}"
avg_duration_ms=0
if [[ ${total_fixtures} -gt 0 ]]; then
  avg_duration_ms="$(( total_duration_ms / total_fixtures ))"
fi

cp tests/parity/tool-runtime-corpus.json "${artifact_dir}/tool-runtime-corpus.json"

cat > "${metrics_file}" <<EOF
{
  "gate": "cp3",
  "passed": ${passed},
  "totalFixtures": ${total_fixtures},
  "totalDurationMs": ${total_duration_ms},
  "avgFixtureDurationMs": ${avg_duration_ms},
  "resultsTsv": "$(basename "${results_file}")"
}
EOF

cat > "${summary_file}" <<EOF
## CP3 Tool Runtime Parity Gate

- Fixtures passed: ${passed}/${total_fixtures}
- Total duration: ${total_duration_ms} ms
- Avg fixture duration: ${avg_duration_ms} ms
- Artifact log: $(basename "${log_file}")
- Artifact metrics: $(basename "${metrics_file}")
- Fixture corpus: tool-runtime-corpus.json
EOF

echo "[parity] CP3 gate passed" | tee -a "${log_file}"
