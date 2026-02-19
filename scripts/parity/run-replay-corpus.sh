#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

echo "[parity] running replay test: protocol_corpus_snapshot_matches_expectations"
cargo test protocol_corpus_snapshot_matches_expectations -- --nocapture

echo "[parity] running replay test: dispatcher_payload_corpus_matches_upstream_fixtures"
cargo test dispatcher_payload_corpus_matches_upstream_fixtures -- --nocapture

echo "[parity] replay corpus suite passed"
