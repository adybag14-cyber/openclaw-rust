import asyncio
import json
import os
import time

import websockets


GATEWAY_URL = os.getenv("PARITY_GATEWAY_URL", "ws://gateway:8765/ws")
GATEWAY_TOKEN = os.getenv("PARITY_GATEWAY_TOKEN", "")
ACTION_ID = os.getenv("PARITY_ACTION_ID", "parity-action-1")
EXPECT_ACTION = os.getenv("PARITY_EXPECT_ACTION", "block").strip().lower()
ASSERT_TIMEOUT_SECS = float(os.getenv("PARITY_ASSERT_TIMEOUT_SECS", "45"))
SCENARIO_JSON = os.getenv("PARITY_SCENARIO_JSON", "").strip()


def connect_frame(client_name: str) -> dict:
    return {
        "type": "req",
        "id": f"connect-{client_name}",
        "method": "connect",
        "params": {
            "client": client_name,
            "role": "client",
            "auth": {"token": GATEWAY_TOKEN},
        },
    }


def parse_scenario() -> list[dict]:
    if not SCENARIO_JSON:
        return [{"id": ACTION_ID, "expected_action": EXPECT_ACTION}]

    parsed = json.loads(SCENARIO_JSON)
    if not isinstance(parsed, list):
        raise ValueError("PARITY_SCENARIO_JSON must be a JSON array")

    events: list[dict] = []
    for idx, item in enumerate(parsed):
        if not isinstance(item, dict):
            raise ValueError(f"PARITY_SCENARIO_JSON[{idx}] must be an object")
        request_id = (
            str(
                item.get("id")
                or item.get("requestId")
                or item.get("actionId")
                or ""
            )
            .strip()
        )
        if not request_id:
            raise ValueError(f"PARITY_SCENARIO_JSON[{idx}] missing id")
        expected_action = (
            str(item.get("expectedAction") or item.get("expectAction") or EXPECT_ACTION)
            .strip()
            .lower()
        )
        if expected_action not in {"allow", "review", "block"}:
            raise ValueError(
                f"PARITY_SCENARIO_JSON[{idx}] invalid expected action `{expected_action}`"
            )
        events.append({"id": request_id, "expected_action": expected_action})

    if not events:
        raise ValueError("PARITY_SCENARIO_JSON must contain at least one event")
    return events


async def wait_for_response(
    ws: websockets.WebSocketClientProtocol, request_id: str, timeout: float
) -> dict:
    deadline = time.monotonic() + timeout
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError(f"timed out waiting for response {request_id}")
        raw = await asyncio.wait_for(ws.recv(), timeout=remaining)
        frame = json.loads(raw)
        if (
            isinstance(frame, dict)
            and frame.get("type") == "resp"
            and str(frame.get("id")) == request_id
        ):
            return frame


def extract_decision(frame_raw: str) -> tuple[str, str] | None:
    frame = json.loads(frame_raw)
    if not isinstance(frame, dict):
        return None
    if frame.get("type") != "event" or frame.get("event") != "security.decision":
        return None
    payload = frame.get("payload")
    if not isinstance(payload, dict):
        return None
    request_id = str(payload.get("requestId", "")).strip()
    if not request_id:
        return None
    decision = payload.get("decision")
    if not isinstance(decision, dict):
        return None
    action = str(decision.get("action", "")).strip().lower()
    if action not in {"allow", "review", "block"}:
        return None
    return request_id, action


async def wait_for_decisions(
    ws: websockets.WebSocketClientProtocol, expected_events: list[dict]
) -> dict[str, str]:
    expected = {entry["id"]: entry["expected_action"] for entry in expected_events}
    pending = expected.copy()
    seen: dict[str, str] = {}

    deadline = time.monotonic() + ASSERT_TIMEOUT_SECS
    while pending:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            missing = ", ".join(sorted(pending.keys()))
            raise TimeoutError(f"timed out waiting for decision events: missing={missing}")

        frame_raw = await asyncio.wait_for(ws.recv(), timeout=remaining)
        extracted = extract_decision(frame_raw)
        if extracted is None:
            continue

        request_id, action = extracted
        if request_id in seen:
            raise RuntimeError(f"duplicate decision received for request {request_id}")
        if request_id not in pending:
            continue

        expected_action = pending[request_id]
        if action != expected_action:
            raise RuntimeError(
                f"unexpected decision action for {request_id}: expected={expected_action}, actual={action}"
            )
        seen[request_id] = action
        del pending[request_id]

    # Ensure there is no immediate duplicate decision for already-consumed ids.
    duplicate_check_deadline = time.monotonic() + 0.35
    while True:
        remaining = duplicate_check_deadline - time.monotonic()
        if remaining <= 0:
            break
        try:
            frame_raw = await asyncio.wait_for(ws.recv(), timeout=remaining)
        except TimeoutError:
            break
        extracted = extract_decision(frame_raw)
        if extracted is None:
            continue
        request_id, _ = extracted
        if request_id in seen:
            raise RuntimeError(f"duplicate decision received for request {request_id}")

    return seen


async def main() -> None:
    scenario = parse_scenario()
    async with websockets.connect(GATEWAY_URL, open_timeout=10, close_timeout=5) as ws:
        connect = connect_frame("parity-assertor")
        await ws.send(json.dumps(connect))
        connect_resp = await wait_for_response(ws, connect["id"], timeout=10.0)
        if not connect_resp.get("ok", False):
            raise RuntimeError(f"assertor connect rejected: {connect_resp}")

        seen = await wait_for_decisions(ws, scenario)
        expected_ids = [entry["id"] for entry in scenario]
        ordered = [f"{request_id}:{seen[request_id]}" for request_id in expected_ids]
        print(f"assertor: decisions matched ({', '.join(ordered)})")


if __name__ == "__main__":
    asyncio.run(main())
