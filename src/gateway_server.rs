use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::time::{timeout, MissedTickBehavior};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

use crate::channels::DriverRegistry;
use crate::config::{
    Config, GatewayAuthMode, GatewayConfig, GroupActivationMode, SessionQueueMode,
};
use crate::gateway::{supported_rpc_methods, RpcDispatchOutcome, RpcDispatcher};
use crate::protocol::{
    decision_event_frame, frame_kind, parse_frame_text, parse_rpc_request,
    rpc_error_response_frame, rpc_success_response_frame, FrameKind, RpcRequestFrame,
};
use crate::scheduler::{SessionScheduler, SessionSchedulerConfig, SubmitOutcome};
use crate::security::ActionEvaluator;
use crate::types::{ActionRequest, Decision};

const ADMIN_SCOPE: &str = "operator.admin";
const APPROVALS_SCOPE: &str = "operator.approvals";
const PAIRING_SCOPE: &str = "operator.pairing";
const READ_SCOPE: &str = "operator.read";
const WRITE_SCOPE: &str = "operator.write";

const NODE_ROLE_METHODS: &[&str] = &["node.invoke.result", "node.event", "skills.bins"];
const APPROVAL_METHODS: &[&str] = &[
    "exec.approval.request",
    "exec.approval.waitdecision",
    "exec.approval.resolve",
];
const PAIRING_METHODS: &[&str] = &[
    "node.pair.request",
    "node.pair.list",
    "node.pair.approve",
    "node.pair.reject",
    "node.pair.verify",
    "device.pair.list",
    "device.pair.approve",
    "device.pair.reject",
    "device.pair.remove",
    "device.token.rotate",
    "device.token.revoke",
    "node.rename",
];
const READ_METHODS: &[&str] = &[
    "health",
    "logs.tail",
    "channels.status",
    "status",
    "usage.status",
    "usage.cost",
    "tts.status",
    "tts.providers",
    "models.list",
    "agents.list",
    "agent.identity.get",
    "skills.status",
    "voicewake.get",
    "sessions.list",
    "sessions.preview",
    "cron.list",
    "cron.status",
    "cron.runs",
    "system-presence",
    "last-heartbeat",
    "node.list",
    "node.describe",
    "chat.history",
    "config.get",
    "config.schema",
    "talk.config",
];
const WRITE_METHODS: &[&str] = &[
    "send",
    "agent",
    "agent.wait",
    "wake",
    "talk.mode",
    "tts.enable",
    "tts.disable",
    "tts.convert",
    "tts.setprovider",
    "voicewake.set",
    "node.invoke",
    "chat.send",
    "chat.abort",
    "browser.request",
    "browser.open",
    "canvas.present",
    "web.login.start",
    "web.login.wait",
    "wizard.start",
    "wizard.next",
    "wizard.cancel",
    "wizard.status",
    "config.set",
    "config.patch",
    "config.apply",
];

const EVENT_SCOPE_APPROVALS: &[&str] = &["exec.approval.requested", "exec.approval.resolved"];
const EVENT_SCOPE_PAIRING: &[&str] = &[
    "device.pair.requested",
    "device.pair.resolved",
    "node.pair.requested",
    "node.pair.resolved",
];
const CRON_DUE_TICK_INTERVAL_MS: u64 = 250;
const CRON_DUE_MAX_BATCH: usize = 32;

#[derive(Clone)]
pub struct GatewayServer {
    gateway: GatewayConfig,
    decision_event: String,
    scheduler_cfg: SessionSchedulerConfig,
    rpc: Arc<RpcDispatcher>,
    drivers: Arc<DriverRegistry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedGatewayAuth {
    None,
    Token(String),
    Password(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveServerSettings {
    auth: ResolvedGatewayAuth,
    handshake_timeout_ms: u64,
    event_queue_capacity: usize,
}

#[derive(Debug, Clone)]
struct ConnectedClient {
    role: String,
    scopes: Vec<String>,
    tx: mpsc::Sender<Message>,
}

struct ServerState {
    decision_event: String,
    rpc: Arc<RpcDispatcher>,
    drivers: Arc<DriverRegistry>,
    scheduler: Arc<SessionScheduler>,
    inflight: Arc<Semaphore>,
    evaluator: Arc<dyn ActionEvaluator>,
    broadcaster: GatewayBroadcaster,
}

#[derive(Debug, Clone)]
struct GatewayBroadcaster {
    clients: Arc<Mutex<HashMap<String, ConnectedClient>>>,
    seq: Arc<AtomicU64>,
}

#[derive(Debug, Deserialize, Default)]
struct ConnectAuthPayload {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ConnectClientPayload {
    #[serde(default)]
    id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ConnectParamsPayload {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    scopes: Option<Vec<String>>,
    #[serde(default)]
    auth: Option<ConnectAuthPayload>,
    #[serde(default)]
    client: Option<ConnectClientPayload>,
}

impl GatewayServer {
    pub fn new(
        gateway: GatewayConfig,
        decision_event: String,
        max_queue: usize,
        queue_mode: SessionQueueMode,
        group_activation_mode: GroupActivationMode,
    ) -> Self {
        Self {
            gateway,
            decision_event,
            scheduler_cfg: SessionSchedulerConfig::new(
                max_queue.max(16),
                queue_mode,
                group_activation_mode,
            ),
            rpc: Arc::new(RpcDispatcher::new()),
            drivers: Arc::new(DriverRegistry::default_registry()),
        }
    }

    pub async fn run_forever(
        &self,
        evaluator: Arc<dyn ActionEvaluator>,
        config_path: Option<PathBuf>,
    ) -> Result<()> {
        self.run_until(evaluator, config_path, std::future::pending::<()>())
            .await
    }

    pub async fn run_until<F>(
        &self,
        evaluator: Arc<dyn ActionEvaluator>,
        config_path: Option<PathBuf>,
        shutdown: F,
    ) -> Result<()>
    where
        F: Future<Output = ()> + Send,
    {
        let listener = TcpListener::bind(&self.gateway.server.bind)
            .await
            .with_context(|| {
                format!(
                    "failed binding standalone gateway listener on {}",
                    self.gateway.server.bind
                )
            })?;
        let bound_addr = listener
            .local_addr()
            .context("failed reading bound address")?;
        info!("standalone gateway listening on ws://{bound_addr}");

        let live_settings = Arc::new(Mutex::new(LiveServerSettings::from_gateway(&self.gateway)));
        let reload_task = self.spawn_live_reload_task(config_path, live_settings.clone());
        let state = Arc::new(ServerState {
            decision_event: self.decision_event.clone(),
            rpc: self.rpc.clone(),
            drivers: self.drivers.clone(),
            scheduler: Arc::new(SessionScheduler::new(self.scheduler_cfg)),
            inflight: Arc::new(Semaphore::new(self.scheduler_cfg.max_pending)),
            evaluator,
            broadcaster: GatewayBroadcaster::new(),
        });
        let cron_due_task = self.spawn_cron_due_task(state.rpc.clone());
        let http_task = self.spawn_control_http_task(state.rpc.clone());

        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = &mut shutdown => {
                    break;
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, remote_addr)) => {
                            let state = state.clone();
                            let live_settings = live_settings.clone();
                            tokio::spawn(async move {
                                if let Err(err) = handle_connection(stream, remote_addr, state, live_settings).await {
                                    warn!("standalone gateway connection failed: {err:#}");
                                }
                            });
                        }
                        Err(err) => {
                            warn!("standalone gateway accept failed: {err}");
                        }
                    }
                }
            }
        }

        if let Some(task) = reload_task {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = http_task {
            task.abort();
            let _ = task.await;
        }
        cron_due_task.abort();
        let _ = cron_due_task.await;
        Ok(())
    }

    fn spawn_live_reload_task(
        &self,
        config_path: Option<PathBuf>,
        live_settings: Arc<Mutex<LiveServerSettings>>,
    ) -> Option<tokio::task::JoinHandle<()>> {
        let interval_secs = self.gateway.server.reload_interval_secs;
        let path = config_path?;
        if interval_secs == 0 {
            return None;
        }
        Some(tokio::spawn(async move {
            let interval = Duration::from_secs(interval_secs.max(1));
            let mut previous_settings: Option<LiveServerSettings> = None;
            loop {
                tokio::time::sleep(interval).await;
                match Config::load(&path) {
                    Ok(cfg) => {
                        let next = LiveServerSettings::from_gateway(&cfg.gateway);
                        if previous_settings.as_ref() == Some(&next) {
                            continue;
                        }
                        previous_settings = Some(next.clone());
                        let mut guard = live_settings.lock().await;
                        *guard = next;
                        info!("gateway live-reload applied from {}", path.display());
                    }
                    Err(err) => {
                        warn!("gateway live-reload skipped (invalid config): {err:#}");
                    }
                }
            }
        }))
    }

    fn spawn_cron_due_task(&self, rpc: Arc<RpcDispatcher>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_millis(CRON_DUE_TICK_INTERVAL_MS));
            interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                let executed = rpc.run_due_cron_jobs(CRON_DUE_MAX_BATCH).await;
                if executed > 0 {
                    debug!("standalone gateway auto-ran {executed} due cron jobs");
                }
            }
        })
    }

    fn spawn_control_http_task(
        &self,
        rpc: Arc<RpcDispatcher>,
    ) -> Option<tokio::task::JoinHandle<()>> {
        let bind = self.gateway.server.http_bind.clone()?;
        if bind.trim().is_empty() {
            return None;
        }
        Some(tokio::spawn(async move {
            let listener = match TcpListener::bind(&bind).await {
                Ok(listener) => listener,
                Err(err) => {
                    warn!("standalone gateway control-http bind failed on {bind}: {err}");
                    return;
                }
            };
            let bound = listener
                .local_addr()
                .map(|addr| addr.to_string())
                .unwrap_or(bind.clone());
            info!("standalone gateway control-http listening on http://{bound}");
            loop {
                match listener.accept().await {
                    Ok((stream, remote_addr)) => {
                        let rpc = rpc.clone();
                        tokio::spawn(async move {
                            if let Err(err) = handle_control_http_connection(stream, rpc).await {
                                warn!(
                                    "standalone gateway control-http connection {} failed: {err}",
                                    remote_addr
                                );
                            }
                        });
                    }
                    Err(err) => {
                        warn!("standalone gateway control-http accept failed: {err}");
                    }
                }
            }
        }))
    }
}

impl LiveServerSettings {
    fn from_gateway(gateway: &GatewayConfig) -> Self {
        Self {
            auth: resolve_gateway_auth(gateway),
            handshake_timeout_ms: gateway.server.handshake_timeout_ms.max(500),
            event_queue_capacity: gateway.server.event_queue_capacity.max(8),
        }
    }
}

impl GatewayBroadcaster {
    fn new() -> Self {
        Self {
            clients: Arc::new(Mutex::new(HashMap::new())),
            seq: Arc::new(AtomicU64::new(0)),
        }
    }

    async fn register(
        &self,
        conn_id: String,
        role: String,
        scopes: Vec<String>,
        tx: mpsc::Sender<Message>,
    ) {
        let mut guard = self.clients.lock().await;
        guard.insert(conn_id, ConnectedClient { role, scopes, tx });
    }

    async fn unregister(&self, conn_id: &str) {
        let mut guard = self.clients.lock().await;
        guard.remove(conn_id);
    }

    #[cfg(test)]
    async fn client_count(&self) -> usize {
        let guard = self.clients.lock().await;
        guard.len()
    }

    async fn broadcast_event(&self, event: &str, payload: Value, drop_if_slow: bool) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let frame = json!({
            "type": "event",
            "event": event,
            "payload": payload,
            "seq": seq,
        });
        let frame_text = frame.to_string();
        let mut stale = Vec::new();
        let mut guard = self.clients.lock().await;
        for (conn_id, client) in guard.iter() {
            if !has_event_scope(event, &client.role, &client.scopes) {
                continue;
            }
            match client.tx.try_send(Message::Text(frame_text.clone())) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    stale.push(conn_id.clone());
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    if drop_if_slow {
                        continue;
                    }
                    stale.push(conn_id.clone());
                }
            }
        }
        for conn_id in stale {
            guard.remove(&conn_id);
        }
    }
}

async fn handle_control_http_connection(
    mut stream: tokio::net::TcpStream,
    rpc: Arc<RpcDispatcher>,
) -> Result<()> {
    let mut buffer = vec![0_u8; 16 * 1024];
    let read = stream
        .read(&mut buffer)
        .await
        .context("failed reading control-http request")?;
    if read == 0 {
        return Ok(());
    }
    let request = String::from_utf8_lossy(&buffer[..read]);
    let request_line = request.lines().next().unwrap_or_default();
    let mut segments = request_line.split_whitespace();
    let method = segments.next().unwrap_or_default();
    let path = segments.next().unwrap_or("/");
    let path = path.split('?').next().unwrap_or(path);

    if method != "GET" {
        let body = json!({
            "ok": false,
            "error": "method_not_allowed"
        });
        return write_http_json_response(&mut stream, 405, &body).await;
    }

    match path {
        "/" | "/ui" | "/control" => {
            write_http_html_response(&mut stream, 200, control_ui_html().as_bytes()).await
        }
        "/health" => {
            let req = RpcRequestFrame {
                id: "http-health".to_owned(),
                method: "health".to_owned(),
                params: json!({}),
            };
            let payload = dispatch_http_rpc(&rpc, req).await;
            write_http_json_response(&mut stream, 200, &payload).await
        }
        "/status" => {
            let req = RpcRequestFrame {
                id: "http-status".to_owned(),
                method: "status".to_owned(),
                params: json!({}),
            };
            let payload = dispatch_http_rpc(&rpc, req).await;
            write_http_json_response(&mut stream, 200, &payload).await
        }
        "/rpc/methods" => {
            let methods = supported_rpc_methods()
                .iter()
                .map(|method| Value::String((*method).to_owned()))
                .collect::<Vec<_>>();
            let payload = json!({
                "ok": true,
                "count": methods.len(),
                "methods": methods
            });
            write_http_json_response(&mut stream, 200, &payload).await
        }
        _ => {
            let payload = json!({
                "ok": false,
                "error": "not_found",
                "path": path
            });
            write_http_json_response(&mut stream, 404, &payload).await
        }
    }
}

async fn dispatch_http_rpc(rpc: &RpcDispatcher, request: RpcRequestFrame) -> Value {
    match rpc.handle_request(&request).await {
        RpcDispatchOutcome::Handled(payload) => json!({
            "ok": true,
            "result": payload
        }),
        RpcDispatchOutcome::Error {
            code,
            message,
            details,
        } => json!({
            "ok": false,
            "error": {
                "code": code,
                "message": message,
                "details": details
            }
        }),
        RpcDispatchOutcome::NotHandled => json!({
            "ok": true,
            "notHandled": true
        }),
    }
}

async fn write_http_json_response(
    stream: &mut tokio::net::TcpStream,
    status_code: u16,
    payload: &Value,
) -> Result<()> {
    let body = serde_json::to_vec(payload).context("failed serializing control-http JSON body")?;
    write_http_response(
        stream,
        status_code,
        "application/json; charset=utf-8",
        &body,
    )
    .await
}

async fn write_http_html_response(
    stream: &mut tokio::net::TcpStream,
    status_code: u16,
    body: &[u8],
) -> Result<()> {
    write_http_response(stream, status_code, "text/html; charset=utf-8", body).await
}

async fn write_http_response(
    stream: &mut tokio::net::TcpStream,
    status_code: u16,
    content_type: &str,
    body: &[u8],
) -> Result<()> {
    let status_text = match status_code {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "OK",
    };
    let head = format!(
        "HTTP/1.1 {status_code} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    );
    stream
        .write_all(head.as_bytes())
        .await
        .context("failed writing control-http headers")?;
    stream
        .write_all(body)
        .await
        .context("failed writing control-http body")?;
    let _ = stream.shutdown().await;
    Ok(())
}

fn control_ui_html() -> String {
    let html = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>OpenClaw Rust Control</title>
    <style>
      :root { --bg:#0f172a; --fg:#e2e8f0; --muted:#94a3b8; --acc:#22d3ee; --panel:#111827; }
      body { margin:0; font-family: ui-sans-serif,system-ui,-apple-system; background: radial-gradient(circle at top,#1e293b 0%,#0f172a 45%,#020617 100%); color:var(--fg); }
      main { max-width: 980px; margin: 2rem auto; padding: 1.25rem; }
      .panel { background: rgba(17,24,39,0.88); border: 1px solid rgba(148,163,184,0.2); border-radius: 14px; padding: 1rem; margin-bottom: 1rem; }
      h1 { margin: 0 0 1rem 0; font-size: 1.4rem; letter-spacing: 0.01em; }
      h2 { margin: 0 0 .6rem 0; font-size: 1rem; color: var(--acc); }
      pre { margin: 0; white-space: pre-wrap; color: var(--muted); font-size: .85rem; }
      .hint { color: var(--muted); font-size: .85rem; margin-top: .5rem; }
    </style>
  </head>
  <body>
    <main>
      <h1>OpenClaw Rust Control Surface</h1>
      <div class="panel">
        <h2>Health</h2>
        <pre id="health">loading...</pre>
      </div>
      <div class="panel">
        <h2>Status</h2>
        <pre id="status">loading...</pre>
      </div>
      <div class="panel">
        <h2>RPC Methods</h2>
        <pre id="methods">loading...</pre>
      </div>
      <div class="hint">This UI is served by the Rust standalone gateway HTTP control surface.</div>
    </main>
    <script>
      async function load(id, url) {
        const el = document.getElementById(id);
        try {
          const res = await fetch(url, { cache: "no-store" });
          const data = await res.json();
          el.textContent = JSON.stringify(data, null, 2);
        } catch (err) {
          el.textContent = String(err);
        }
      }
      load("health", "/health");
      load("status", "/status");
      load("methods", "/rpc/methods");
    </script>
  </body>
</html>"#;
    html.to_owned()
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    remote_addr: std::net::SocketAddr,
    state: Arc<ServerState>,
    live_settings: Arc<Mutex<LiveServerSettings>>,
) -> Result<()> {
    let ws = accept_async(stream)
        .await
        .with_context(|| format!("websocket upgrade failed for {remote_addr}"))?;
    let conn_id = format!("conn-{}", now_ms());
    let (mut write, mut read) = ws.split();

    let challenge = json!({
        "type": "event",
        "event": "connect.challenge",
        "payload": {
            "nonce": format!("nonce-{}", now_ms()),
            "ts": now_ms()
        }
    });
    write
        .send(Message::Text(challenge.to_string()))
        .await
        .context("failed sending connect challenge")?;

    let settings = { live_settings.lock().await.clone() };
    let inbound = timeout(
        Duration::from_millis(settings.handshake_timeout_ms),
        read.next(),
    )
    .await
    .context("connect handshake timed out")?
    .ok_or_else(|| anyhow::anyhow!("connection closed before connect handshake"))?
    .context("failed receiving connect handshake frame")?;
    let handshake_text = inbound
        .to_text()
        .context("connect handshake frame must be text")?;
    let handshake_frame = parse_frame_text(handshake_text).context("invalid connect JSON frame")?;
    let connect_req = parse_rpc_request(&handshake_frame)
        .ok_or_else(|| anyhow::anyhow!("connect handshake must be rpc request frame"))?;
    if normalize_method(&connect_req.method) != "connect" {
        let response = rpc_error_response_frame(
            &connect_req.id,
            400,
            "invalid handshake: first request must be connect",
            None,
        );
        write.send(Message::Text(response.to_string())).await?;
        write
            .send(Message::Close(Some(close_frame(
                1008,
                "invalid handshake: first request must be connect",
            ))))
            .await?;
        return Ok(());
    }

    let connect_params: ConnectParamsPayload =
        serde_json::from_value(connect_req.params.clone())
            .map_err(|err| anyhow::anyhow!("invalid connect params: {err}"))?;
    let role = normalize_role(connect_params.role.as_deref())
        .ok_or_else(|| anyhow::anyhow!("invalid role"))?;
    let scopes = normalize_scopes(connect_params.scopes);
    if let Err(reason) = authorize_connect(&settings.auth, connect_params.auth.as_ref()) {
        let response = rpc_error_response_frame(&connect_req.id, 400, reason, None);
        write.send(Message::Text(response.to_string())).await?;
        write
            .send(Message::Close(Some(close_frame(1008, reason))))
            .await?;
        return Ok(());
    }

    let (out_tx, mut out_rx) = mpsc::channel::<Message>(settings.event_queue_capacity.max(8));
    let writer = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            if write.send(message).await.is_err() {
                break;
            }
        }
    });

    state
        .broadcaster
        .register(
            conn_id.clone(),
            role.clone(),
            scopes.clone(),
            out_tx.clone(),
        )
        .await;

    let hello = json!({
        "type": "hello-ok",
        "protocol": 1,
        "server": {
            "version": env!("CARGO_PKG_VERSION"),
            "runtime": "openclaw-agent-rs",
            "connId": conn_id,
            "host": remote_addr.ip().to_string(),
            "clientId": connect_params
                .client
                .as_ref()
                .and_then(|client| client.id.as_deref())
                .unwrap_or("unknown"),
        },
        "features": {
            "methods": supported_rpc_methods(),
            "events": ["heartbeat", "presence", "security.decision"]
        },
        "policy": {
            "maxPayload": 25 * 1024 * 1024,
            "maxBufferedBytes": 50 * 1024 * 1024,
            "tickIntervalMs": 30_000
        }
    });
    let hello_resp = rpc_success_response_frame(&connect_req.id, hello);
    out_tx.send(Message::Text(hello_resp.to_string())).await?;

    info!(
        "standalone gateway connected conn_id={} role={} scopes={}",
        conn_id,
        role,
        scopes.join(",")
    );

    while let Some(inbound) = read.next().await {
        let inbound = inbound.context("websocket inbound error")?;
        match inbound {
            Message::Text(text) => {
                let frame = match parse_frame_text(&text) {
                    Ok(v) => v,
                    Err(err) => {
                        warn!("invalid JSON frame on {conn_id}: {err}");
                        continue;
                    }
                };

                match frame_kind(&frame) {
                    FrameKind::Req => {
                        let Some(req) = parse_rpc_request(&frame) else {
                            continue;
                        };
                        let method = normalize_method(&req.method);
                        if let Some(message) = authorize_gateway_method(&method, &role, &scopes) {
                            let response = rpc_error_response_frame(&req.id, 400, &message, None);
                            let _ = out_tx.send(Message::Text(response.to_string())).await;
                            continue;
                        }

                        let response = match state.rpc.handle_request(&req).await {
                            RpcDispatchOutcome::Handled(result) => {
                                rpc_success_response_frame(&req.id, result)
                            }
                            RpcDispatchOutcome::Error {
                                code,
                                message,
                                details,
                            } => rpc_error_response_frame(&req.id, code, &message, details),
                            RpcDispatchOutcome::NotHandled => rpc_error_response_frame(
                                &req.id,
                                400,
                                &format!("unknown method: {}", req.method),
                                None,
                            ),
                        };
                        let _ = out_tx.send(Message::Text(response.to_string())).await;
                    }
                    FrameKind::Event => {
                        state.rpc.ingest_event_frame(&frame).await;
                        if let Some(event) = frame.get("event").and_then(Value::as_str) {
                            let payload = frame.get("payload").cloned().unwrap_or(Value::Null);
                            let drop_if_slow = matches!(
                                normalize_method(event).as_str(),
                                "heartbeat" | "presence" | "tick"
                            );
                            state
                                .broadcaster
                                .broadcast_event(event, payload, drop_if_slow)
                                .await;
                        }

                        if let Some(request) = state.drivers.extract(&frame) {
                            match state.scheduler.submit(request).await {
                                SubmitOutcome::Dispatch(dispatch_request) => {
                                    let Ok(slot) = state.inflight.clone().try_acquire_owned()
                                    else {
                                        warn!(
                                            "standalone gateway decision queue saturated, dropping request {}",
                                            dispatch_request.id
                                        );
                                        let _ = state.scheduler.complete(&dispatch_request).await;
                                        continue;
                                    };
                                    spawn_session_worker(dispatch_request, slot, state.clone());
                                }
                                SubmitOutcome::Queued => {}
                                SubmitOutcome::Dropped {
                                    request_id,
                                    session_id,
                                } => {
                                    debug!(
                                        "standalone gateway session queue dropped request {} (session={})",
                                        request_id,
                                        session_id
                                    );
                                }
                                SubmitOutcome::IgnoredActivation {
                                    request_id,
                                    session_id,
                                } => {
                                    debug!(
                                        "standalone gateway ignored request {} due to activation (session={})",
                                        request_id,
                                        session_id
                                    );
                                }
                            }
                        }
                    }
                    FrameKind::Resp | FrameKind::Error | FrameKind::Unknown => {}
                }
            }
            Message::Ping(payload) => {
                let _ = out_tx.try_send(Message::Pong(payload));
            }
            Message::Close(_) => break,
            Message::Binary(_) | Message::Pong(_) | Message::Frame(_) => {}
        }
    }

    state.broadcaster.unregister(&conn_id).await;
    drop(out_tx);
    let _ = writer.await;
    Ok(())
}

fn spawn_session_worker(
    request: ActionRequest,
    slot: OwnedSemaphorePermit,
    state: Arc<ServerState>,
) {
    tokio::spawn(async move {
        let _permit = slot;
        let mut current = request;
        loop {
            let decision = state.evaluator.evaluate(current.clone()).await;
            state.rpc.record_decision(&current, &decision).await;
            broadcast_decision(&state, &current, &decision).await;
            match state.scheduler.complete(&current).await {
                Some(next) => {
                    current = next;
                }
                None => break,
            }
        }
    });
}

async fn broadcast_decision(state: &ServerState, request: &ActionRequest, decision: &Decision) {
    let frame = decision_event_frame(&state.decision_event, request, decision);
    let event = frame
        .get("event")
        .and_then(Value::as_str)
        .unwrap_or(&state.decision_event);
    let payload = frame.get("payload").cloned().unwrap_or(Value::Null);
    state
        .broadcaster
        .broadcast_event(event, payload, true)
        .await;
}

fn resolve_gateway_auth(gateway: &GatewayConfig) -> ResolvedGatewayAuth {
    let mode = gateway.server.auth_mode;
    let token = gateway
        .token
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    let password = gateway
        .password
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    match mode {
        GatewayAuthMode::None => ResolvedGatewayAuth::None,
        GatewayAuthMode::Token => {
            if token.is_empty() {
                ResolvedGatewayAuth::None
            } else {
                ResolvedGatewayAuth::Token(token)
            }
        }
        GatewayAuthMode::Password => {
            if password.is_empty() {
                ResolvedGatewayAuth::None
            } else {
                ResolvedGatewayAuth::Password(password)
            }
        }
        GatewayAuthMode::Auto => {
            if !password.is_empty() {
                ResolvedGatewayAuth::Password(password)
            } else if !token.is_empty() {
                ResolvedGatewayAuth::Token(token)
            } else {
                ResolvedGatewayAuth::None
            }
        }
    }
}

fn authorize_connect(
    auth: &ResolvedGatewayAuth,
    provided: Option<&ConnectAuthPayload>,
) -> std::result::Result<(), &'static str> {
    match auth {
        ResolvedGatewayAuth::None => Ok(()),
        ResolvedGatewayAuth::Token(expected) => {
            let provided = provided
                .and_then(|p| p.token.as_deref())
                .map(str::trim)
                .unwrap_or_default();
            if provided.is_empty() {
                return Err("missing gateway token");
            }
            if provided != expected {
                return Err("gateway token mismatch");
            }
            Ok(())
        }
        ResolvedGatewayAuth::Password(expected) => {
            let provided = provided
                .and_then(|p| p.password.as_deref())
                .map(str::trim)
                .unwrap_or_default();
            if provided.is_empty() {
                return Err("missing gateway password");
            }
            if provided != expected {
                return Err("gateway password mismatch");
            }
            Ok(())
        }
    }
}

fn authorize_gateway_method(method: &str, role: &str, scopes: &[String]) -> Option<String> {
    if method == "connect" {
        return None;
    }
    if NODE_ROLE_METHODS.contains(&method) {
        if role == "node" {
            return None;
        }
        return Some(format!("unauthorized role: {}", role));
    }
    if role == "node" {
        return Some(format!("unauthorized role: {}", role));
    }
    if role != "operator" {
        return Some(format!("unauthorized role: {}", role));
    }
    if has_scope(scopes, ADMIN_SCOPE) {
        return None;
    }
    if APPROVAL_METHODS.contains(&method) && !has_scope(scopes, APPROVALS_SCOPE) {
        return Some(format!("missing scope: {}", APPROVALS_SCOPE));
    }
    if PAIRING_METHODS.contains(&method) && !has_scope(scopes, PAIRING_SCOPE) {
        return Some(format!("missing scope: {}", PAIRING_SCOPE));
    }
    if READ_METHODS.contains(&method)
        && !(has_scope(scopes, READ_SCOPE) || has_scope(scopes, WRITE_SCOPE))
    {
        return Some(format!("missing scope: {}", READ_SCOPE));
    }
    if WRITE_METHODS.contains(&method) && !has_scope(scopes, WRITE_SCOPE) {
        return Some(format!("missing scope: {}", WRITE_SCOPE));
    }
    if APPROVAL_METHODS.contains(&method)
        || PAIRING_METHODS.contains(&method)
        || READ_METHODS.contains(&method)
        || WRITE_METHODS.contains(&method)
    {
        return None;
    }
    if method.starts_with("exec.approvals.") {
        return Some(format!("missing scope: {}", ADMIN_SCOPE));
    }
    if method.starts_with("config.")
        || method.starts_with("wizard.")
        || method.starts_with("update.")
        || method == "channels.logout"
        || method == "agents.create"
        || method == "agents.update"
        || method == "agents.delete"
        || method == "skills.install"
        || method == "skills.update"
        || method == "cron.add"
        || method == "cron.update"
        || method == "cron.remove"
        || method == "cron.run"
        || method == "sessions.patch"
        || method == "sessions.reset"
        || method == "sessions.delete"
        || method == "sessions.compact"
    {
        return Some(format!("missing scope: {}", ADMIN_SCOPE));
    }
    Some(format!("missing scope: {}", ADMIN_SCOPE))
}

fn has_event_scope(event: &str, role: &str, scopes: &[String]) -> bool {
    if role != "operator" {
        return false;
    }
    if has_scope(scopes, ADMIN_SCOPE) {
        return true;
    }
    let event = normalize_method(event);
    if EVENT_SCOPE_APPROVALS.contains(&event.as_str()) {
        return has_scope(scopes, APPROVALS_SCOPE);
    }
    if EVENT_SCOPE_PAIRING.contains(&event.as_str()) {
        return has_scope(scopes, PAIRING_SCOPE);
    }
    true
}

fn has_scope(scopes: &[String], expected: &str) -> bool {
    scopes
        .iter()
        .any(|scope| scope.eq_ignore_ascii_case(expected))
}

fn normalize_role(role: Option<&str>) -> Option<String> {
    let normalized = role.unwrap_or("operator").trim().to_ascii_lowercase();
    match normalized.as_str() {
        "operator" | "node" => Some(normalized),
        _ => None,
    }
}

fn normalize_method(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_scopes(scopes: Option<Vec<String>>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(values) = scopes {
        for scope in values {
            let trimmed = scope.trim();
            if trimmed.is_empty() {
                continue;
            }
            if out
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(trimmed))
            {
                continue;
            }
            out.push(trimmed.to_owned());
        }
    }
    out
}

fn close_frame(code: u16, reason: &'static str) -> CloseFrame<'static> {
    CloseFrame {
        code: CloseCode::from(code),
        reason: reason.into(),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use anyhow::Result;
    use async_trait::async_trait;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::sync::{mpsc, oneshot};
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message;

    use crate::config::{
        GatewayAuthMode, GatewayConfig, GatewayRuntimeMode, GatewayServerConfig,
        GroupActivationMode, SessionQueueMode,
    };
    use crate::security::ActionEvaluator;
    use crate::types::{ActionRequest, Decision, DecisionAction};

    use super::{authorize_gateway_method, GatewayBroadcaster, GatewayServer};

    struct AllowEvaluator;

    #[async_trait]
    impl ActionEvaluator for AllowEvaluator {
        async fn evaluate(&self, _request: ActionRequest) -> Decision {
            Decision {
                action: DecisionAction::Allow,
                risk_score: 0,
                reasons: vec!["ok".to_owned()],
                tags: vec![],
                source: "stub".to_owned(),
            }
        }
    }

    fn reserve_bind() -> Result<String> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        drop(listener);
        Ok(addr.to_string())
    }

    fn test_gateway(bind: String) -> GatewayConfig {
        GatewayConfig {
            url: "ws://127.0.0.1:18789/ws".to_owned(),
            token: Some("cp1-token".to_owned()),
            password: None,
            runtime_mode: GatewayRuntimeMode::StandaloneServer,
            server: GatewayServerConfig {
                bind,
                http_bind: None,
                auth_mode: GatewayAuthMode::Token,
                handshake_timeout_ms: 3_000,
                event_queue_capacity: 8,
                reload_interval_secs: 0,
            },
        }
    }

    async fn ws_connect(
        url: &str,
        role: &str,
        scopes: &[&str],
        token: &str,
    ) -> Result<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    > {
        let (mut ws, _) = connect_async(url).await?;
        let challenge = ws
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("missing challenge"))??;
        let challenge_frame: Value = serde_json::from_str(challenge.to_text()?)?;
        assert_eq!(
            challenge_frame.get("event").and_then(Value::as_str),
            Some("connect.challenge")
        );

        let connect_req = json!({
            "type": "req",
            "id": "connect-1",
            "method": "connect",
            "params": {
                "client": { "id": "control-ui", "version": "1.0.0", "platform": "test", "mode": "desktop" },
                "role": role,
                "scopes": scopes,
                "auth": { "token": token }
            }
        });
        ws.send(Message::Text(connect_req.to_string())).await?;
        let connect_resp = ws
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("missing connect resp"))??;
        let connect_json: Value = serde_json::from_str(connect_resp.to_text()?)?;
        assert_eq!(
            connect_json.pointer("/ok").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            connect_json.pointer("/result/type").and_then(Value::as_str),
            Some("hello-ok")
        );
        Ok(ws)
    }

    async fn http_get_json(bind: &str, path: &str) -> Result<Value> {
        let mut stream = TcpStream::connect(bind).await?;
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {bind}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).await?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await?;
        let text = String::from_utf8(raw)?;
        let body = text
            .split("\r\n\r\n")
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("missing HTTP body"))?;
        Ok(serde_json::from_str(body)?)
    }

    #[test]
    fn authorize_gateway_method_allows_write_scope_for_control_ui_orchestration_methods() {
        let scopes = vec!["operator.write".to_owned()];
        for method in [
            "browser.open",
            "canvas.present",
            "web.login.start",
            "wizard.start",
            "config.patch",
            "config.apply",
        ] {
            assert_eq!(
                authorize_gateway_method(method, "operator", &scopes),
                None,
                "method {method} should be writable by operator.write scope"
            );
        }
    }

    #[tokio::test]
    async fn standalone_gateway_serves_control_plane_rpcs_without_upstream_runtime() -> Result<()> {
        let bind = reserve_bind()?;
        let gateway = test_gateway(bind.clone());
        let server = GatewayServer::new(
            gateway,
            "security.decision".to_owned(),
            64,
            SessionQueueMode::Followup,
            GroupActivationMode::Always,
        );
        let evaluator: Arc<dyn ActionEvaluator> = Arc::new(AllowEvaluator);
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            server
                .run_until(evaluator, None, async {
                    let _ = shutdown_rx.await;
                })
                .await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        let url = format!("ws://{bind}");
        let mut ws = ws_connect(&url, "operator", &["operator.admin"], "cp1-token").await?;

        let req = json!({
            "type": "req",
            "id": "health-1",
            "method": "health",
            "params": {}
        });
        ws.send(Message::Text(req.to_string())).await?;
        let resp = ws
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("missing health response"))??;
        let resp_json: Value = serde_json::from_str(resp.to_text()?)?;
        assert_eq!(
            resp_json.pointer("/ok").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            resp_json.pointer("/id").and_then(Value::as_str),
            Some("health-1")
        );

        let _ = shutdown_tx.send(());
        task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn standalone_gateway_control_http_serves_health_status_and_methods() -> Result<()> {
        let ws_bind = reserve_bind()?;
        let http_bind = reserve_bind()?;
        let mut gateway = test_gateway(ws_bind);
        gateway.server.http_bind = Some(http_bind.clone());
        let server = GatewayServer::new(
            gateway,
            "security.decision".to_owned(),
            64,
            SessionQueueMode::Followup,
            GroupActivationMode::Always,
        );
        let evaluator: Arc<dyn ActionEvaluator> = Arc::new(AllowEvaluator);
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            server
                .run_until(evaluator, None, async {
                    let _ = shutdown_rx.await;
                })
                .await
        });

        tokio::time::sleep(Duration::from_millis(60)).await;
        let health = http_get_json(&http_bind, "/health").await?;
        assert_eq!(health.pointer("/ok").and_then(Value::as_bool), Some(true));
        assert!(health.pointer("/result").is_some());

        let status = http_get_json(&http_bind, "/status").await?;
        assert_eq!(status.pointer("/ok").and_then(Value::as_bool), Some(true));
        assert!(status.pointer("/result/runtime/version").is_some());

        let methods = http_get_json(&http_bind, "/rpc/methods").await?;
        assert_eq!(methods.pointer("/ok").and_then(Value::as_bool), Some(true));
        assert!(methods
            .pointer("/count")
            .and_then(Value::as_u64)
            .is_some_and(|count| count > 0));

        let _ = shutdown_tx.send(());
        task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn standalone_gateway_authz_matrix_enforces_roles_and_scopes() -> Result<()> {
        let bind = reserve_bind()?;
        let gateway = test_gateway(bind.clone());
        let server = GatewayServer::new(
            gateway,
            "security.decision".to_owned(),
            64,
            SessionQueueMode::Followup,
            GroupActivationMode::Always,
        );
        let evaluator: Arc<dyn ActionEvaluator> = Arc::new(AllowEvaluator);
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            server
                .run_until(evaluator, None, async {
                    let _ = shutdown_rx.await;
                })
                .await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        let url = format!("ws://{bind}");
        let mut operator_ws = ws_connect(&url, "operator", &["operator.read"], "cp1-token").await?;

        let health_req = json!({"type":"req","id":"read-ok","method":"health","params":{}});
        operator_ws
            .send(Message::Text(health_req.to_string()))
            .await?;
        let health_resp = operator_ws
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("missing health"))??;
        let health_json: Value = serde_json::from_str(health_resp.to_text()?)?;
        assert_eq!(
            health_json.pointer("/ok").and_then(Value::as_bool),
            Some(true)
        );

        let write_req =
            json!({"type":"req","id":"write-deny","method":"chat.send","params":{"text":"hello"}});
        operator_ws
            .send(Message::Text(write_req.to_string()))
            .await?;
        let write_resp = operator_ws
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("missing write deny"))??;
        let write_json: Value = serde_json::from_str(write_resp.to_text()?)?;
        assert_eq!(
            write_json.pointer("/ok").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            write_json
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            "missing scope: operator.write"
        );

        let pairing_req =
            json!({"type":"req","id":"pair-deny","method":"device.pair.list","params":{}});
        operator_ws
            .send(Message::Text(pairing_req.to_string()))
            .await?;
        let pairing_resp = operator_ws
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("missing pairing deny"))??;
        let pairing_json: Value = serde_json::from_str(pairing_resp.to_text()?)?;
        assert_eq!(
            pairing_json.pointer("/ok").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            pairing_json
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            "missing scope: operator.pairing"
        );

        let mut node_ws = ws_connect(&url, "node", &[], "cp1-token").await?;
        let node_health = json!({"type":"req","id":"node-health","method":"health","params":{}});
        node_ws.send(Message::Text(node_health.to_string())).await?;
        let node_health_resp = node_ws
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("missing node health response"))??;
        let node_health_json: Value = serde_json::from_str(node_health_resp.to_text()?)?;
        assert_eq!(
            node_health_json.pointer("/ok").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            node_health_json
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            "unauthorized role: node"
        );

        let node_allowed =
            json!({"type":"req","id":"node-bins","method":"skills.bins","params":{}});
        node_ws
            .send(Message::Text(node_allowed.to_string()))
            .await?;
        let node_allowed_resp = node_ws
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("missing node allowed response"))??;
        let node_allowed_json: Value = serde_json::from_str(node_allowed_resp.to_text()?)?;
        assert_eq!(
            node_allowed_json.pointer("/id").and_then(Value::as_str),
            Some("node-bins")
        );

        let _ = shutdown_tx.send(());
        task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn standalone_gateway_runs_due_cron_jobs_automatically() -> Result<()> {
        let bind = reserve_bind()?;
        let gateway = test_gateway(bind.clone());
        let server = GatewayServer::new(
            gateway,
            "security.decision".to_owned(),
            64,
            SessionQueueMode::Followup,
            GroupActivationMode::Always,
        );
        let evaluator: Arc<dyn ActionEvaluator> = Arc::new(AllowEvaluator);
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            server
                .run_until(evaluator, None, async {
                    let _ = shutdown_rx.await;
                })
                .await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        let url = format!("ws://{bind}");
        let mut ws = ws_connect(&url, "operator", &["operator.admin"], "cp1-token").await?;

        let add_req = json!({
            "type": "req",
            "id": "cron-add-auto-1",
            "method": "cron.add",
            "params": {
                "name": "Auto due cron",
                "schedule": {
                    "kind": "every",
                    "everyMs": 60_000
                },
                "sessionTarget": "main",
                "wakeMode": "next-heartbeat",
                "payload": {
                    "kind": "systemEvent",
                    "text": "cron auto due"
                }
            }
        });
        ws.send(Message::Text(add_req.to_string())).await?;
        let add_resp = ws
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("missing cron.add response"))??;
        let add_json: Value = serde_json::from_str(add_resp.to_text()?)?;
        let job_id = add_json
            .pointer("/result/id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| anyhow::anyhow!("missing cron job id"))?;

        let mut observed = false;
        for _ in 0..20 {
            let runs_req = json!({
                "type": "req",
                "id": "cron-runs-auto-1",
                "method": "cron.runs",
                "params": {
                    "id": job_id.clone(),
                    "limit": 5
                }
            });
            ws.send(Message::Text(runs_req.to_string())).await?;
            let runs_resp = ws
                .next()
                .await
                .ok_or_else(|| anyhow::anyhow!("missing cron.runs response"))??;
            let runs_json: Value = serde_json::from_str(runs_resp.to_text()?)?;
            let entries = runs_json
                .pointer("/result/entries")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if !entries.is_empty() {
                observed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            observed,
            "expected due cron job to run automatically in standalone mode"
        );

        let _ = shutdown_tx.send(());
        task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn broadcaster_backpressure_drop_if_slow_semantics() -> Result<()> {
        let broadcaster = GatewayBroadcaster::new();
        let (fast_tx, mut fast_rx) = mpsc::channel::<Message>(1);
        let (slow_tx, mut slow_rx) = mpsc::channel::<Message>(1);

        broadcaster
            .register(
                "fast".to_owned(),
                "operator".to_owned(),
                vec!["operator.admin".to_owned()],
                fast_tx,
            )
            .await;
        broadcaster
            .register(
                "slow".to_owned(),
                "operator".to_owned(),
                vec!["operator.admin".to_owned()],
                slow_tx,
            )
            .await;

        broadcaster
            .broadcast_event("heartbeat", json!({"seq": 1}), false)
            .await;
        let first_fast = fast_rx
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("missing first fast frame"))?;
        let first_fast_json: Value = serde_json::from_str(first_fast.to_text()?)?;
        assert_eq!(
            first_fast_json
                .pointer("/payload/seq")
                .and_then(Value::as_i64),
            Some(1)
        );

        broadcaster
            .broadcast_event("heartbeat", json!({"seq": 2}), true)
            .await;
        let second_fast = fast_rx
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("missing second fast frame"))?;
        let second_fast_json: Value = serde_json::from_str(second_fast.to_text()?)?;
        assert_eq!(
            second_fast_json
                .pointer("/payload/seq")
                .and_then(Value::as_i64),
            Some(2)
        );

        broadcaster
            .broadcast_event("heartbeat", json!({"seq": 3}), false)
            .await;
        let third_fast = fast_rx
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("missing third fast frame"))?;
        let third_fast_json: Value = serde_json::from_str(third_fast.to_text()?)?;
        assert_eq!(
            third_fast_json
                .pointer("/payload/seq")
                .and_then(Value::as_i64),
            Some(3)
        );

        let slow_first = slow_rx
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("missing slow frame"))?;
        let slow_first_json: Value = serde_json::from_str(slow_first.to_text()?)?;
        assert_eq!(
            slow_first_json
                .pointer("/payload/seq")
                .and_then(Value::as_i64),
            Some(1)
        );
        let slow_next = tokio::time::timeout(Duration::from_millis(80), slow_rx.recv()).await;
        match slow_next {
            Err(_) => {}
            Ok(None) => {}
            Ok(Some(message)) => {
                let frame: Value = serde_json::from_str(message.to_text()?)?;
                panic!("slow consumer unexpectedly received frame: {}", frame);
            }
        }
        assert_eq!(broadcaster.client_count().await, 1);
        Ok(())
    }
}
