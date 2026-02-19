#!/usr/bin/env bash
set -euo pipefail

artifact_dir="${PARITY_ARTIFACT_DIR:-parity/generated/cp5}"
mkdir -p "${artifact_dir}"

log_file="${artifact_dir}/cp5-gate.log"
results_file="${artifact_dir}/cp5-fixture-results.tsv"
summary_file="${artifact_dir}/cp5-gate-summary.md"
metrics_file="${artifact_dir}/cp5-metrics.json"

tests=(
  "gateway::tests::dispatcher_browser_request_validates_and_reports_unavailable_contract"
  "gateway::tests::dispatcher_browser_request_routes_through_node_proxy_runtime"
  "gateway::tests::dispatcher_browser_request_enforces_browser_proxy_command_allowlist"
  "gateway::tests::dispatcher_browser_open_routes_through_browser_proxy_runtime"
  "gateway::tests::dispatcher_canvas_present_routes_through_node_runtime"
  "gateway::tests::dispatcher_canvas_present_rejects_disallowed_command"
  "gateway::tests::dispatcher_device_pair_and_token_methods_follow_parity_contract"
  "gateway::tests::dispatcher_node_pairing_methods_follow_parity_contract"
  "gateway::tests::dispatcher_node_invoke_and_event_methods_follow_parity_contract"
  "gateway::tests::dispatcher_node_invoke_supports_camera_screen_location_and_system_commands_when_declared"
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
  echo "[parity] running CP5 fixture: ${test_name}" | tee -a "${log_file}"
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
    echo "[parity] CP5 fixture failed: ${test_name}" | tee -a "${log_file}"
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
  "gate": "cp5",
  "passed": ${passed},
  "totalFixtures": ${total_fixtures},
  "totalDurationMs": ${total_duration_ms},
  "avgFixtureDurationMs": ${avg_duration_ms},
  "resultsTsv": "$(basename "${results_file}")"
}
EOF

cat > "${summary_file}" <<EOF
## CP5 Nodes + Browser + Canvas + Device Gate

- Fixtures passed: ${passed}/${total_fixtures}
- Total duration: ${total_duration_ms} ms
- Avg fixture duration: ${avg_duration_ms} ms
- Artifact log: $(basename "${log_file}")
- Artifact metrics: $(basename "${metrics_file}")
EOF

echo "[parity] CP5 gate passed" | tee -a "${log_file}"
