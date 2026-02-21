#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
IMAGE_TAG="openclaw-rs-parity-runtime:latest"

docker build -f "${ROOT_DIR}/deploy/Dockerfile.parity-runtime" -t "${IMAGE_TAG}" "${ROOT_DIR}"
docker run --rm --entrypoint /usr/local/bin/openclaw-agent-rs "${IMAGE_TAG}" --help >/dev/null
