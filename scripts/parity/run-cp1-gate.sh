#!/usr/bin/env bash
set -euo pipefail

tests=(
  "gateway_server::tests::standalone_gateway_serves_control_plane_rpcs_without_upstream_runtime"
  "gateway_server::tests::standalone_gateway_authz_matrix_enforces_roles_and_scopes"
  "gateway_server::tests::broadcaster_backpressure_drop_if_slow_semantics"
  "gateway_server::tests::channel_webhook_route_aliases_are_supported"
  "gateway_server::tests::standalone_gateway_control_http_webhook_batch_ingest_dispatches_all_decisions"
)

for test_name in "${tests[@]}"; do
  echo "[parity] running CP1 fixture: ${test_name}"
  cargo test "${test_name}" -- --nocapture
done

echo "[parity] CP1 gate passed"
