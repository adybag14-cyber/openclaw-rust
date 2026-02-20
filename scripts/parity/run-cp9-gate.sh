#!/usr/bin/env bash
set -euo pipefail

PARITY_ARTIFACT_DIR="${PARITY_ARTIFACT_DIR:-parity/generated/cp9}"

mkdir -p "${PARITY_ARTIFACT_DIR}"

log_file="${PARITY_ARTIFACT_DIR}/cp9-gate.log"
results_file="${PARITY_ARTIFACT_DIR}/cp9-check-results.tsv"
summary_file="${PARITY_ARTIFACT_DIR}/cp9-gate-summary.md"
metrics_file="${PARITY_ARTIFACT_DIR}/cp9-metrics.json"

: > "${log_file}"
echo -e "check\tduration_ms\tstatus" > "${results_file}"

total_duration_ms=0
checks_run=0
passed=0
overall_status="pass"
docker_server_version="unavailable"
docker_compose_version="unavailable"

run_check() {
  local check_name="$1"
  shift

  local start_ms
  start_ms="$(date +%s%3N)"
  checks_run=$((checks_run + 1))

  echo "[parity] running CP9 check: ${check_name}" | tee -a "${log_file}"
  if ! "$@" 2>&1 | tee -a "${log_file}"; then
    local end_ms
    end_ms="$(date +%s%3N)"
    local duration_ms=$((end_ms - start_ms))
    total_duration_ms=$((total_duration_ms + duration_ms))
    echo -e "${check_name}\t${duration_ms}\tfail" >> "${results_file}"
    overall_status="fail"
    return 1
  fi

  local end_ms
  end_ms="$(date +%s%3N)"
  local duration_ms=$((end_ms - start_ms))
  total_duration_ms=$((total_duration_ms + duration_ms))
  passed=$((passed + 1))
  echo -e "${check_name}\t${duration_ms}\tpass" >> "${results_file}"
  return 0
}

if run_check "docker-daemon" docker info; then
  docker_server_version="$(docker version --format '{{.Server.Version}}' 2>/dev/null || echo "unknown")"
  docker_compose_version="$(docker compose version --short 2>/dev/null || docker compose version 2>/dev/null || echo "unknown")"

  run_check "docker-smoke" bash ./scripts/run-docker-parity-smoke.sh || true
  if [[ "${overall_status}" == "pass" ]]; then
    run_check "docker-compose-parity" bash ./scripts/run-docker-compose-parity.sh || true
  fi
  if [[ "${overall_status}" == "pass" ]]; then
    run_check "docker-compose-chaos-restart" bash ./scripts/run-docker-compose-parity-chaos.sh || true
  fi
fi

failed=$((checks_run - passed))
avg_duration_ms=0
if [[ "${checks_run}" -gt 0 ]]; then
  avg_duration_ms=$((total_duration_ms / checks_run))
fi

cat > "${summary_file}" <<EOF
## CP9 Docker End-to-End Parity Gate

- Checks passed: ${passed}/${checks_run}
- Checks failed: ${failed}
- Total duration: ${total_duration_ms} ms
- Avg check duration: ${avg_duration_ms} ms
- Docker server version: ${docker_server_version}
- Docker compose version: ${docker_compose_version}
- Artifact log: cp9-gate.log
- Artifact metrics: cp9-metrics.json
- Artifact results: cp9-check-results.tsv
EOF

cat > "${metrics_file}" <<EOF
{
  "gate": "cp9",
  "status": "${overall_status}",
  "checksRun": ${checks_run},
  "checksPassed": ${passed},
  "checksFailed": ${failed},
  "totalDurationMs": ${total_duration_ms},
  "avgCheckDurationMs": ${avg_duration_ms},
  "dockerServerVersion": "${docker_server_version}",
  "dockerComposeVersion": "${docker_compose_version}",
  "resultsTsv": "cp9-check-results.tsv"
}
EOF

if [[ "${overall_status}" != "pass" ]]; then
  echo "[parity] CP9 gate failed" | tee -a "${log_file}"
  exit 1
fi

echo "[parity] CP9 gate passed" | tee -a "${log_file}"
