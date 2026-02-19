#!/usr/bin/env bash
set -euo pipefail

artifact_dir="${PARITY_ARTIFACT_DIR:-parity/generated/cp2}"
mkdir -p "${artifact_dir}"

log_file="${artifact_dir}/cp2-gate.log"
results_file="${artifact_dir}/cp2-fixture-results.tsv"
summary_file="${artifact_dir}/cp2-gate-summary.md"
metrics_file="${artifact_dir}/cp2-metrics.json"

echo -e "suite\ttest\tduration_ms\tstatus" > "${results_file}"
: > "${log_file}"

default_tests=(
  "bridge::tests::steer_mode_keeps_latest_pending_at_bridge_level"
  "bridge::tests::followup_queue_pressure_preserves_order_without_duplicates"
  "bridge::tests::session_routing_corpus_matches_expected_delivery_order"
  "bridge::tests::multi_session_soak_preserves_per_session_fifo_without_duplicates"
  "bridge::tests::reply_back_payload_preserves_group_and_direct_delivery_context"
  "gateway::tests::dispatcher_list_supports_label_spawn_filters_and_message_hints"
  "gateway::tests::dispatcher_list_route_selectors_disambiguate_shared_peer_by_account_and_channel"
  "gateway::tests::dispatcher_resolve_supports_label_agent_and_spawn_filters"
  "gateway::tests::dispatcher_resolve_route_selectors_disambiguate_shared_peer_by_account_and_channel"
  "gateway::tests::dispatcher_resolve_prefers_explicit_session_key_over_route_selectors"
  "gateway::tests::dispatcher_resolve_prefers_session_id_over_label_and_route_selectors"
  "gateway::tests::dispatcher_resolve_supports_label_plus_route_selectors"
  "gateway::tests::dispatcher_resolve_accepts_partial_route_selectors_without_account_id"
  "gateway::tests::dispatcher_resolve_partial_route_collision_prefers_most_recent_update"
  "gateway::tests::dispatcher_resolve_partial_route_collision_uses_key_tiebreak_when_timestamps_match"
)

sqlite_tests=(
  "state::tests::sqlite_state_survives_restart_and_continues_counters"
  "state::tests::sqlite_state_recovers_multiple_sessions_after_restart"
)

now_ms() {
  local ms
  ms="$(date +%s%3N 2>/dev/null || true)"
  if [[ -n "${ms}" ]]; then
    echo "${ms}"
    return
  fi
  echo "$(( $(date +%s) * 1000 ))"
}

default_passed=0
sqlite_passed=0
total_duration_ms=0
soak_count=0
soak_duration_ms=0

run_fixture() {
  local suite="$1"
  local test_name="$2"
  local cargo_args="$3"
  local start_ms end_ms duration_ms
  start_ms="$(now_ms)"

  echo "[parity] running CP2 ${suite} fixture: ${test_name}" | tee -a "${log_file}"
  if cargo test ${cargo_args} "${test_name}" -- --nocapture 2>&1 | tee -a "${log_file}"; then
    :
  else
    end_ms="$(now_ms)"
    duration_ms="$(( end_ms - start_ms ))"
    echo -e "${suite}\t${test_name}\t${duration_ms}\tfail" >> "${results_file}"
    echo "[parity] CP2 ${suite} fixture failed: ${test_name}" | tee -a "${log_file}"
    exit 1
  fi
  end_ms="$(now_ms)"
  duration_ms="$(( end_ms - start_ms ))"
  total_duration_ms="$(( total_duration_ms + duration_ms ))"
  if [[ "${test_name}" == *"soak"* || "${test_name}" == *"queue_pressure"* || "${test_name}" == *"delivery_order"* ]]; then
    soak_count="$(( soak_count + 1 ))"
    soak_duration_ms="$(( soak_duration_ms + duration_ms ))"
  fi
  echo -e "${suite}\t${test_name}\t${duration_ms}\tpass" >> "${results_file}"
}

for test_name in "${default_tests[@]}"; do
  run_fixture "default" "${test_name}" ""
  default_passed="$(( default_passed + 1 ))"
done

for test_name in "${sqlite_tests[@]}"; do
  run_fixture "sqlite-feature" "${test_name}" "--features sqlite-state"
  sqlite_passed="$(( sqlite_passed + 1 ))"
done

total_fixtures="$(( default_passed + sqlite_passed ))"
avg_duration_ms=0
if [[ ${total_fixtures} -gt 0 ]]; then
  avg_duration_ms="$(( total_duration_ms / total_fixtures ))"
fi

cp tests/parity/session-routing-corpus.json "${artifact_dir}/session-routing-corpus.json"
cp tests/parity/gateway-payload-corpus.json "${artifact_dir}/gateway-payload-corpus.json"

cat > "${metrics_file}" <<EOF
{
  "gate": "cp2",
  "defaultPassed": ${default_passed},
  "sqliteFeaturePassed": ${sqlite_passed},
  "totalFixtures": ${total_fixtures},
  "totalDurationMs": ${total_duration_ms},
  "avgFixtureDurationMs": ${avg_duration_ms},
  "soakFixtureCount": ${soak_count},
  "soakFixtureDurationMs": ${soak_duration_ms},
  "resultsTsv": "$(basename "${results_file}")"
}
EOF

cat > "${summary_file}" <<EOF
## CP2 Session/Routing Gate

- Default fixtures passed: ${default_passed}
- SQLite feature fixtures passed: ${sqlite_passed}
- Total fixtures: ${total_fixtures}
- Total duration: ${total_duration_ms} ms
- Avg fixture duration: ${avg_duration_ms} ms
- Soak/order fixtures: ${soak_count}
- Soak/order fixture duration: ${soak_duration_ms} ms
- Artifact log: $(basename "${log_file}")
- Artifact metrics: $(basename "${metrics_file}")
EOF

echo "[parity] CP2 gate passed" | tee -a "${log_file}"
