import asyncio
import json
import logging
import os
from dataclasses import dataclass
from typing import Any

from websockets.exceptions import ConnectionClosed
from websockets.server import WebSocketServerProtocol, serve


logging.basicConfig(
    level=os.getenv("PARITY_GATEWAY_LOG_LEVEL", "INFO"),
    format="%(asctime)s %(levelname)s gateway-mock %(message)s",
)
LOGGER = logging.getLogger("gateway-mock")

HOST = os.getenv("PARITY_GATEWAY_HOST", "0.0.0.0")
PORT = int(os.getenv("PARITY_GATEWAY_PORT", "8765"))
TOKEN = os.getenv("PARITY_GATEWAY_TOKEN", "")


def make_response(req_id: str, ok: bool, result: Any = None, error: Any = None) -> str:
    payload: dict[str, Any] = {"type": "resp", "id": req_id, "ok": ok}
    if ok:
        payload["result"] = result if result is not None else {}
    else:
        payload["error"] = error if error is not None else {"code": 500, "message": "unknown"}
    return json.dumps(payload, separators=(",", ":"))


@dataclass
class ClientState:
    client_name: str = "unknown"


class GatewayState:
    def __init__(self) -> None:
        self._clients: dict[WebSocketServerProtocol, ClientState] = {}
        self._lock = asyncio.Lock()

    async def register(self, ws: WebSocketServerProtocol, client_name: str) -> None:
        async with self._lock:
            self._clients[ws] = ClientState(client_name=client_name)
        LOGGER.info("client connected: %s", client_name)

    async def unregister(self, ws: WebSocketServerProtocol) -> None:
        async with self._lock:
            state = self._clients.pop(ws, None)
        if state is not None:
            LOGGER.info("client disconnected: %s", state.client_name)

    async def known_clients(self) -> list[str]:
        async with self._lock:
            return sorted({state.client_name for state in self._clients.values()})

    async def broadcast(self, frame: dict[str, Any], exclude: WebSocketServerProtocol | None) -> None:
        serialized = json.dumps(frame, separators=(",", ":"))
        async with self._lock:
            targets = [ws for ws in self._clients if ws is not exclude]

        stale: list[WebSocketServerProtocol] = []
        for ws in targets:
            try:
                await ws.send(serialized)
            except Exception:
                stale.append(ws)
        for ws in stale:
            await self.unregister(ws)


async def handle_request(
    ws: WebSocketServerProtocol, frame: dict[str, Any], state: GatewayState
) -> None:
    req_id = str(frame.get("id", "unknown"))
    method = str(frame.get("method", "")).strip().lower()
    params = frame.get("params")
    if not isinstance(params, dict):
        params = {}

    if method == "connect":
        auth = params.get("auth")
        token = auth.get("token") if isinstance(auth, dict) else ""
        if TOKEN and token != TOKEN:
            await ws.send(
                make_response(
                    req_id,
                    ok=False,
                    error={"code": 401, "message": "invalid gateway token"},
                )
            )
            await ws.close(code=4001, reason="invalid token")
            return

        client_name = str(params.get("client") or "unknown-client")
        await state.register(ws, client_name)
        await ws.send(
            make_response(
                req_id,
                ok=True,
                result={"ok": True, "client": client_name},
            )
        )
        return

    if method in {"health", "status", "parity.clients"}:
        clients = await state.known_clients()
        result = {"status": "ok", "clientCount": len(clients), "clients": clients}
        await ws.send(make_response(req_id, ok=True, result=result))
        return

    await ws.send(
        make_response(
            req_id,
            ok=False,
            error={"code": 404, "message": f"unsupported method: {method}"},
        )
    )


async def connection_loop(ws: WebSocketServerProtocol, state: GatewayState) -> None:
    try:
        async for raw in ws:
            try:
                frame = json.loads(raw)
            except json.JSONDecodeError:
                LOGGER.warning("received non-json frame")
                continue
            if not isinstance(frame, dict):
                continue

            frame_type = str(frame.get("type", "")).strip().lower()
            if frame_type == "req":
                await handle_request(ws, frame, state)
                continue
            if frame_type == "event":
                event_name = str(frame.get("event", "unknown"))
                LOGGER.info("event inbound: %s", event_name)
                await state.broadcast(frame, exclude=ws)
                continue
    except ConnectionClosed:
        pass
    finally:
        await state.unregister(ws)


async def main() -> None:
    state = GatewayState()
    async with serve(
        lambda ws: connection_loop(ws, state),
        HOST,
        PORT,
        ping_interval=20,
        ping_timeout=20,
    ):
        LOGGER.info("gateway mock listening on ws://%s:%d/ws", HOST, PORT)
        await asyncio.Future()


if __name__ == "__main__":
    asyncio.run(main())
