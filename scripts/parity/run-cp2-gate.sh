#!/usr/bin/env bash
set -euo pipefail

default_tests=(
  "bridge::tests::steer_mode_keeps_latest_pending_at_bridge_level"
  "bridge::tests::followup_queue_pressure_preserves_order_without_duplicates"
  "bridge::tests::session_routing_corpus_matches_expected_delivery_order"
  "bridge::tests::multi_session_soak_preserves_per_session_fifo_without_duplicates"
  "gateway::tests::dispatcher_list_supports_label_spawn_filters_and_message_hints"
  "gateway::tests::dispatcher_resolve_supports_label_agent_and_spawn_filters"
)

sqlite_tests=(
  "state::tests::sqlite_state_survives_restart_and_continues_counters"
  "state::tests::sqlite_state_recovers_multiple_sessions_after_restart"
)

for test_name in "${default_tests[@]}"; do
  echo "[parity] running CP2 fixture: ${test_name}"
  cargo test "${test_name}" -- --nocapture
done

for test_name in "${sqlite_tests[@]}"; do
  echo "[parity] running CP2 sqlite fixture: ${test_name}"
  cargo test --features sqlite-state "${test_name}" -- --nocapture
done

echo "[parity] CP2 gate passed"
