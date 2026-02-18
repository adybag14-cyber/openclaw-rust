#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
COMPOSE_FILE="${ROOT_DIR}/deploy/docker-compose.parity.yml"

if ! docker info >/dev/null 2>&1; then
  echo "docker daemon is not reachable. Start Docker Desktop/service and retry." >&2
  exit 1
fi

cleanup() {
  docker compose -f "${COMPOSE_FILE}" down --volumes --remove-orphans >/dev/null 2>&1 || true
}

trap cleanup EXIT

docker compose -f "${COMPOSE_FILE}" build
docker compose -f "${COMPOSE_FILE}" up --abort-on-container-exit --exit-code-from assertor
