import asyncio
import json
import os
import sys

import websockets


GATEWAY_URL = os.getenv("PARITY_GATEWAY_URL", "ws://localhost:8765/ws")
GATEWAY_TOKEN = os.getenv("PARITY_GATEWAY_TOKEN", "parity-token")


async def main() -> int:
    async with websockets.connect(GATEWAY_URL, open_timeout=3, close_timeout=2) as ws:
        req = {
            "type": "req",
            "id": "connect-parity-healthcheck",
            "method": "connect",
            "params": {
                "client": "parity-healthcheck",
                "role": "client",
                "auth": {"token": GATEWAY_TOKEN},
            },
        }
        await ws.send(json.dumps(req))
        raw = await asyncio.wait_for(ws.recv(), timeout=3)
        frame = json.loads(raw)
        if not isinstance(frame, dict) or frame.get("ok") is not True:
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
