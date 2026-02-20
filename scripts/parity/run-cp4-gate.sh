#!/usr/bin/env bash
set -euo pipefail

CARGO_BIN="${CARGO_BIN:-cargo}"
TOOLCHAIN="${TOOLCHAIN:-}"
artifact_dir="${PARITY_ARTIFACT_DIR:-parity/generated/cp4}"
mkdir -p "${artifact_dir}"

log_file="${artifact_dir}/cp4-gate.log"
results_file="${artifact_dir}/cp4-fixture-results.tsv"
summary_file="${artifact_dir}/cp4-gate-summary.md"
metrics_file="${artifact_dir}/cp4-metrics.json"

tests=(
  "channels::tests::exposes_channel_capabilities_and_wave1_order"
  "channels::tests::signal_driver_detects_source"
  "channels::tests::webchat_driver_detects_source"
  "channels::tests::normalize_chat_type_supports_dm_alias"
  "channels::tests::mention_gate_skips_when_required_and_not_mentioned"
  "channels::tests::mention_gate_with_bypass_allows_authorized_control_commands"
  "channels::tests::chunking_supports_length_and_newline_modes"
  "channels::tests::default_chunk_limit_matches_core_channel_defaults"
  "channels::tests::retry_backoff_policy_scales_and_caps"
  "channels::tests::wave2_drivers_detect_source_aliases"
  "channels::tests::normalize_channel_id_supports_wave2_aliases"
  "channels::tests::wave3_drivers_detect_source_aliases"
  "channels::tests::normalize_channel_id_supports_wave3_aliases"
  "channels::tests::wave4_drivers_detect_source_aliases"
  "channels::tests::normalize_channel_id_supports_wave4_aliases"
  "scheduler::tests::mention_activation_accepts_group_message_when_detection_unavailable"
  "scheduler::tests::mention_activation_bypasses_for_authorized_control_command"
  "gateway::tests::dispatcher_channels_methods_report_status_and_validate_logout"
  "gateway::tests::dispatcher_channels_status_rejects_unknown_params"
  "gateway::tests::dispatcher_channels_status_probe_false_sets_null_channel_last_probe_at"
  "gateway::tests::dispatcher_channels_logout_rejects_unknown_params"
  "gateway::tests::dispatcher_channels_logout_accepts_channel_alias"
  "gateway::tests::dispatcher_channels_status_reflects_runtime_event_snapshots"
  "gateway::tests::dispatcher_channels_status_tracks_payload_channel_alias_runtime"
  "gateway::tests::dispatcher_channels_status_tracks_wave3_payload_channel_alias_runtime"
  "gateway::tests::dispatcher_channels_status_tracks_wave4_payload_channel_alias_runtime"
  "gateway::tests::dispatcher_channels_logout_marks_runtime_offline"
  "gateway::tests::dispatcher_channels_logout_without_runtime_account_does_not_create_account"
  "gateway::tests::dispatcher_channels_status_ingests_channel_accounts_runtime_map"
  "gateway::tests::dispatcher_channels_status_honors_default_account_hints_from_runtime_payload"
  "gateway::tests::dispatcher_channels_status_ingests_nested_default_account_id_from_channels_map"
  "gateway::tests::dispatcher_channels_status_ingests_nested_snake_case_default_account_id_from_channels_map"
  "gateway::tests::dispatcher_channels_status_ingests_alias_channel_ids_in_runtime_maps"
  "gateway::tests::dispatcher_channels_status_includes_wave2_channel_catalog_entries"
  "gateway::tests::dispatcher_channels_status_ingests_wave2_alias_channel_ids_in_runtime_maps"
  "gateway::tests::dispatcher_channels_status_includes_wave3_channel_catalog_entries"
  "gateway::tests::dispatcher_channels_status_ingests_wave3_alias_channel_ids_in_runtime_maps"
  "gateway::tests::dispatcher_channels_status_includes_wave4_channel_catalog_entries"
  "gateway::tests::dispatcher_channels_status_ingests_wave4_alias_channel_ids_in_runtime_maps"
  "gateway::tests::dispatcher_channels_status_ingests_snake_case_runtime_maps"
  "gateway::tests::dispatcher_channels_status_tracks_inbound_when_channel_is_only_in_payload"
  "gateway::tests::dispatcher_chat_send_updates_webchat_runtime_outbound_activity"
  "gateway::tests::dispatcher_channels_status_defaults_to_unconfigured_unlinked_without_runtime"
  "gateway::tests::dispatcher_channels_status_probe_false_sets_null_account_last_probe_at"
  "gateway::tests::dispatcher_channels_status_probe_true_sets_account_last_probe_at"
  "gateway::tests::dispatcher_channels_status_ingests_extended_account_metadata_fields"
  "gateway::tests::dispatcher_channels_status_ingests_runtime_probe_audit_and_application_payloads"
  "gateway::tests::dispatcher_channels_logout_matches_account_id_case_insensitively"
  "gateway::tests::dispatcher_channels_logout_blank_account_id_defaults_default"
  "gateway::tests::dispatcher_channels_status_canonicalizes_default_account_id_casing"
  "gateway::tests::dispatcher_channels_status_orders_default_account_first"
  "gateway::tests::dispatcher_channels_status_ingests_runtime_account_name"
  "gateway::tests::dispatcher_channels_status_ingests_runtime_display_name_alias"
  "gateway::tests::dispatcher_channels_status_ingests_snake_case_extended_metadata_fields"
  "gateway::tests::dispatcher_channels_status_parses_allow_from_string_list"
  "gateway::tests::dispatcher_send_accepts_wave2_channel_aliases"
  "gateway::tests::dispatcher_send_accepts_wave3_channel_aliases"
  "gateway::tests::dispatcher_send_accepts_wave4_channel_aliases"
  "gateway::tests::dispatcher_channels_status_accepts_numeric_channel_default_account_id_map_values"
  "gateway::tests::dispatcher_channels_status_accepts_numeric_payload_default_account_id"
  "gateway::tests::dispatcher_channels_status_accepts_numeric_nested_default_account_id"
  "gateway::tests::dispatcher_channels_status_parses_last_probe_at_from_string_number"
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
  echo "[parity] running CP4 fixture: ${test_name}" | tee -a "${log_file}"
  cmd=("${CARGO_BIN}")
  if [[ -n "${TOOLCHAIN}" ]]; then
    cmd+=("+${TOOLCHAIN}")
  fi
  cmd+=(test "${test_name}" -- --nocapture)
  if "${cmd[@]}" 2>&1 | tee -a "${log_file}"; then
    end_ms="$(now_ms)"
    duration_ms="$(( end_ms - start_ms ))"
    total_duration_ms="$(( total_duration_ms + duration_ms ))"
    echo -e "${test_name}\t${duration_ms}\tpass" >> "${results_file}"
    passed="$(( passed + 1 ))"
  else
    end_ms="$(now_ms)"
    duration_ms="$(( end_ms - start_ms ))"
    echo -e "${test_name}\t${duration_ms}\tfail" >> "${results_file}"
    echo "[parity] CP4 fixture failed: ${test_name}" | tee -a "${log_file}"
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
  "gate": "cp4",
  "passed": ${passed},
  "totalFixtures": ${total_fixtures},
  "totalDurationMs": ${total_duration_ms},
  "avgFixtureDurationMs": ${avg_duration_ms},
  "resultsTsv": "$(basename "${results_file}")"
}
EOF

cat > "${summary_file}" <<EOF
## CP4 Channel Runtime Wave-1/Wave-2/Wave-3/Wave-4 Foundation Gate

- Fixtures passed: ${passed}/${total_fixtures}
- Total duration: ${total_duration_ms} ms
- Avg fixture duration: ${avg_duration_ms} ms
- Artifact log: $(basename "${log_file}")
- Artifact metrics: $(basename "${metrics_file}")
EOF

echo "[parity] CP4 gate passed" | tee -a "${log_file}"
