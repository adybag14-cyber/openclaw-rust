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


async def wait_for_decision(ws: websockets.WebSocketClientProtocol) -> dict:
    deadline = time.monotonic() + ASSERT_TIMEOUT_SECS
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError(f"timed out waiting for decision event for {ACTION_ID}")
        raw = await asyncio.wait_for(ws.recv(), timeout=remaining)
        frame = json.loads(raw)
        if not isinstance(frame, dict):
            continue
        if frame.get("type") != "event":
            continue
        if frame.get("event") != "security.decision":
            continue
        payload = frame.get("payload")
        if not isinstance(payload, dict):
            continue
        if str(payload.get("requestId")) != ACTION_ID:
            continue
        return payload


async def main() -> None:
    async with websockets.connect(GATEWAY_URL, open_timeout=10, close_timeout=5) as ws:
        connect = connect_frame("parity-assertor")
        await ws.send(json.dumps(connect))
        connect_resp = await wait_for_response(ws, connect["id"], timeout=10.0)
        if not connect_resp.get("ok", False):
            raise RuntimeError(f"assertor connect rejected: {connect_resp}")

        payload = await wait_for_decision(ws)
        decision = payload.get("decision")
        if not isinstance(decision, dict):
            raise RuntimeError(f"decision payload malformed: {payload}")
        action = str(decision.get("action", "")).strip().lower()
        if action != EXPECT_ACTION:
            raise RuntimeError(
                f"unexpected decision action for {ACTION_ID}: expected={EXPECT_ACTION}, actual={action}"
            )
        score = decision.get("risk_score")
        print(f"assertor: decision matched for {ACTION_ID} action={action} score={score}")


if __name__ == "__main__":
    asyncio.run(main())
