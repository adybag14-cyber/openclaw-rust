#!/usr/bin/env python3
import argparse
import datetime as dt
import hashlib
import hmac
import json
import os
from pathlib import Path
from typing import Any


def canonicalize(value: Any) -> Any:
    if isinstance(value, dict):
        return {key: canonicalize(value[key]) for key in sorted(value.keys())}
    if isinstance(value, list):
        return [canonicalize(item) for item in value]
    return value


def sign_bundle(unsigned: dict[str, Any], key: str) -> dict[str, Any]:
    canonical = canonicalize(unsigned)
    payload = json.dumps(canonical, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    signature = hmac.new(key.encode("utf-8"), payload, hashlib.sha256).hexdigest()
    signed = dict(unsigned)
    signed["signature"] = signature
    return signed


def load_unsigned_bundle(path: Path) -> dict[str, Any]:
    raw = path.read_text(encoding="utf-8")
    parsed = json.loads(raw)
    if not isinstance(parsed, dict):
        raise ValueError("unsigned bundle root must be a JSON object")
    parsed.pop("signature", None)
    return parsed


def write_bundle(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def resolve_key(explicit: str | None, env_name: str | None, label: str) -> str:
    if explicit and explicit.strip():
        return explicit.strip()
    if env_name:
        candidate = os.getenv(env_name, "").strip()
        if candidate:
            return candidate
    raise ValueError(f"missing {label}; provide explicit value or set {env_name}")


def build_signed_variant(
    unsigned: dict[str, Any],
    key_id: str,
    key: str,
    stage: str,
) -> dict[str, Any]:
    payload = dict(unsigned)
    payload["keyId"] = key_id
    payload["signedAt"] = dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z")
    payload["rolloutStage"] = stage
    return sign_bundle(payload, key)


def cmd_rotate(args: argparse.Namespace) -> int:
    unsigned = load_unsigned_bundle(Path(args.unsigned))
    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    new_key = resolve_key(args.new_key, args.new_key_env, "new key")
    previous_key = None
    if args.previous_key_id:
        previous_key = resolve_key(
            args.previous_key,
            args.previous_key_env,
            "previous key",
        )

    canary = build_signed_variant(unsigned, args.new_key_id, new_key, "canary")
    staged = build_signed_variant(unsigned, args.new_key_id, new_key, "staged")

    canary_path = output_dir / "policy-bundle-canary.json"
    staged_path = output_dir / "policy-bundle-staged.json"
    write_bundle(canary_path, canary)
    write_bundle(staged_path, staged)

    manifest: dict[str, Any] = {
        "generatedAt": dt.datetime.now(dt.timezone.utc)
        .isoformat()
        .replace("+00:00", "Z"),
        "unsignedSource": str(Path(args.unsigned)),
        "newKeyId": args.new_key_id,
        "artifacts": {
            "canary": str(canary_path),
            "staged": str(staged_path),
        },
    }

    if args.previous_key_id:
        rollback = build_signed_variant(
            unsigned,
            args.previous_key_id,
            previous_key or "",
            "rollback",
        )
        rollback_path = output_dir / "policy-bundle-rollback.json"
        write_bundle(rollback_path, rollback)
        manifest["previousKeyId"] = args.previous_key_id
        manifest["artifacts"]["rollback"] = str(rollback_path)

    manifest_path = output_dir / "policy-bundle-rotation-manifest.json"
    write_bundle(manifest_path, manifest)
    print(
        json.dumps(
            {
                "ok": True,
                "manifest": str(manifest_path),
                "artifacts": manifest["artifacts"],
            },
            ensure_ascii=False,
        )
    )
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Generate staged signed policy bundles for key rotation."
    )
    parser.add_argument(
        "--unsigned",
        required=True,
        help="Path to unsigned policy bundle JSON payload.",
    )
    parser.add_argument(
        "--output-dir",
        required=True,
        help="Directory to write signed bundle artifacts.",
    )
    parser.add_argument(
        "--new-key-id",
        required=True,
        help="Key id to embed for new canary/staged bundles.",
    )
    parser.add_argument("--new-key", help="New key material (HMAC SHA-256 secret).")
    parser.add_argument(
        "--new-key-env",
        default="OPENCLAW_RS_POLICY_BUNDLE_KEY",
        help="Env var fallback for new key material.",
    )
    parser.add_argument(
        "--previous-key-id",
        help="Optional previous key id to generate rollback bundle.",
    )
    parser.add_argument(
        "--previous-key",
        help="Optional previous key material for rollback bundle.",
    )
    parser.add_argument(
        "--previous-key-env",
        default="OPENCLAW_RS_POLICY_BUNDLE_PREVIOUS_KEY",
        help="Env var fallback for previous key material.",
    )
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    return cmd_rotate(args)


if __name__ == "__main__":
    raise SystemExit(main())
