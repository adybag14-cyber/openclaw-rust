mod bridge;
mod channels;
mod config;
mod gateway;
mod gateway_server;
mod memory;
mod persistent_memory;
mod protocol;
mod routines;
mod runtime;
mod scheduler;
mod security;
mod security_audit;
mod session_key;
mod state;
mod telegram_bridge;
mod tool_registry;
mod tool_runtime;
mod types;
mod wasm_runtime;
mod wasm_sandbox;
mod website_bridge;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use clap::{Args, Parser, Subcommand};
use config::{Config, GatewayAuthMode, ToolRuntimeWasmMode};
use futures_util::{SinkExt, StreamExt};
use gateway::{RpcDispatchOutcome, RpcDispatcher};
use protocol::RpcRequestFrame;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Clone, Parser)]
#[command(author, version, about = "Rust runtime + defender for OpenClaw")]
struct Cli {
    /// Path to TOML config file.
    #[arg(
        long,
        global = true,
        env = "OPENCLAW_RS_CONFIG",
        default_value = "openclaw-rs.toml"
    )]
    config: PathBuf,

    /// Override gateway URL.
    #[arg(long, global = true, env = "OPENCLAW_RS_GATEWAY_URL")]
    gateway_url: Option<String>,

    /// Override gateway token.
    #[arg(long, global = true, env = "OPENCLAW_RS_GATEWAY_TOKEN")]
    gateway_token: Option<String>,

    /// Enable audit-only mode (never block, always review/allow with annotation).
    #[arg(long, global = true, env = "OPENCLAW_RS_AUDIT_ONLY")]
    audit_only: bool,

    /// Log level filter, e.g. info,debug,trace.
    #[arg(long, global = true, env = "OPENCLAW_RS_LOG", default_value = "info")]
    log: String,

    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Clone, Subcommand)]
enum CliCommand {
    /// Run the Rust OpenClaw runtime.
    Run,
    /// Query runtime status payload.
    Status(StatusArgs),
    /// Query runtime health payload.
    Health(HealthArgs),
    /// Run non-interactive diagnostics for operator parity checks.
    Doctor(DoctorArgs),
    /// Run security audit and remediation parity surface.
    Security(SecurityArgs),
    /// Run gateway command parity surface.
    Gateway(GatewayArgs),
    /// Query tools catalog parity surface.
    Tools(ToolsArgs),
    /// Run agent command parity surface.
    Agent(AgentArgs),
    /// Run message command parity surface.
    Message(MessageArgs),
    /// Run nodes command parity surface.
    Nodes(NodesArgs),
    /// Run sessions command parity surface.
    Sessions(SessionsArgs),
}

#[derive(Debug, Clone, Args, Default)]
struct DoctorArgs {
    /// Disable prompts and print deterministic diagnostics.
    #[arg(long)]
    non_interactive: bool,
    /// Emit doctor output as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Args, Default)]
struct StatusArgs {
    /// Emit output as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Args, Default)]
struct HealthArgs {
    /// Emit output as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Args, Default)]
struct SecurityArgs {
    #[command(subcommand)]
    command: Option<SecuritySubcommand>,
}

#[derive(Debug, Clone, Subcommand)]
enum SecuritySubcommand {
    /// Audit config + local state for common security foot-guns.
    Audit(SecurityAuditArgs),
}

#[derive(Debug, Clone, Args, Default)]
struct SecurityAuditArgs {
    /// Attempt best-effort live gateway reachability probe.
    #[arg(long)]
    deep: bool,
    /// Apply deterministic safe remediations and permissions tightening.
    #[arg(long)]
    fix: bool,
    /// Emit output as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Args, Default)]
struct GatewayArgs {
    #[command(subcommand)]
    command: Option<GatewaySubcommand>,
    /// Emit output as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Subcommand)]
enum GatewaySubcommand {
    /// Run the runtime (alias of top-level run).
    Run,
    /// Query gateway runtime status payload.
    Status,
    /// Query gateway health payload.
    Health,
    /// List supported RPC methods.
    Methods,
    /// Call any RPC method directly.
    Call(GatewayCallArgs),
}

#[derive(Debug, Clone, Args)]
struct GatewayCallArgs {
    /// RPC method name.
    #[arg(long)]
    method: String,
    /// JSON payload for params.
    #[arg(long, default_value = "{}")]
    params: String,
    /// Route the call through the configured live gateway websocket service.
    #[arg(long)]
    live_service: bool,
    /// Emit output as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Args, Default)]
struct ToolsArgs {
    #[command(subcommand)]
    command: Option<ToolsSubcommand>,
    /// Emit output as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Subcommand)]
enum ToolsSubcommand {
    /// List available tools grouped by section/profile.
    Catalog(ToolsCatalogCommandArgs),
}

#[derive(Debug, Clone, Args, Default)]
struct ToolsCatalogCommandArgs {
    /// Optional explicit agent id.
    #[arg(long = "agent-id")]
    agent_id: Option<String>,
    /// Include plugin tools in output.
    #[arg(long = "include-plugins")]
    include_plugins: Option<bool>,
    /// Emit output as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Args)]
struct AgentArgs {
    /// Prompt/message to send to the agent runtime.
    #[arg(long)]
    message: String,
    /// Optional explicit agent id.
    #[arg(long = "agent-id")]
    agent_id: Option<String>,
    /// Session key for agent context.
    #[arg(long = "session-key", default_value = "main")]
    session_key: String,
    /// Optional delivery channel.
    #[arg(long)]
    channel: Option<String>,
    /// Optional delivery target.
    #[arg(long)]
    to: Option<String>,
    /// Optional delivery account id.
    #[arg(long = "account-id")]
    account_id: Option<String>,
    /// Optional delivery thread id.
    #[arg(long = "thread-id")]
    thread_id: Option<String>,
    /// Optional thinking level hint.
    #[arg(long)]
    thinking: Option<String>,
    /// Wait for completion via `agent.wait`.
    #[arg(long)]
    wait: bool,
    /// Wait timeout in milliseconds when --wait is set.
    #[arg(long = "timeout-ms", default_value_t = 30_000)]
    timeout_ms: u64,
    /// Optional idempotency key; auto-generated when omitted.
    #[arg(long = "idempotency-key")]
    idempotency_key: Option<String>,
    /// Emit output as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Args)]
struct MessageArgs {
    #[command(subcommand)]
    command: MessageSubcommand,
}

#[derive(Debug, Clone, Subcommand)]
enum MessageSubcommand {
    /// Send outbound message payload through gateway parity path.
    Send(MessageSendArgs),
}

#[derive(Debug, Clone, Args)]
struct MessageSendArgs {
    /// Delivery target (for example +1234567890, @user, channel id).
    #[arg(long)]
    to: String,
    /// Optional text body.
    #[arg(long)]
    message: Option<String>,
    /// Optional media URL(s) to send.
    #[arg(long = "media-url")]
    media_urls: Vec<String>,
    /// Optional target channel (default: whatsapp).
    #[arg(long)]
    channel: Option<String>,
    /// Optional account id for provider/account routing.
    #[arg(long = "account-id")]
    account_id: Option<String>,
    /// Optional thread id for threaded providers.
    #[arg(long = "thread-id")]
    thread_id: Option<String>,
    /// Optional session key to mirror send transcript.
    #[arg(long = "session-key")]
    session_key: Option<String>,
    /// Optional idempotency key; auto-generated when omitted.
    #[arg(long = "idempotency-key")]
    idempotency_key: Option<String>,
    /// Emit output as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Args, Default)]
struct NodesArgs {
    #[command(subcommand)]
    command: Option<NodesSubcommand>,
    /// Emit output as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Subcommand)]
enum NodesSubcommand {
    /// List paired/known nodes.
    List,
}

#[derive(Debug, Clone, Args)]
struct SessionsArgs {
    #[command(subcommand)]
    command: SessionsSubcommand,
}

#[derive(Debug, Clone, Subcommand)]
enum SessionsSubcommand {
    /// List session summaries.
    List(SessionsListArgs),
    /// Query one session status snapshot.
    Status(SessionStatusArgs),
}

#[derive(Debug, Clone, Args, Default)]
struct SessionsListArgs {
    /// Max session records.
    #[arg(long)]
    limit: Option<usize>,
    /// Optional agent id filter.
    #[arg(long = "agent-id")]
    agent_id: Option<String>,
    /// Optional channel filter.
    #[arg(long)]
    channel: Option<String>,
    /// Optional text search filter.
    #[arg(long)]
    search: Option<String>,
    /// Include derived titles in response.
    #[arg(long = "include-derived-titles")]
    include_derived_titles: bool,
    /// Include last message in response.
    #[arg(long = "include-last-message")]
    include_last_message: bool,
    /// Emit output as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Args, Default)]
struct SessionStatusArgs {
    /// Session key to inspect.
    #[arg(long = "session-key", default_value = "main")]
    session_key: String,
    /// Emit output as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Serialize)]
struct DoctorReport {
    ok: bool,
    checks: Vec<DoctorCheck>,
}

#[derive(Debug, Clone, Serialize)]
struct DoctorCheck {
    id: String,
    status: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_logging(&cli.log)?;

    let command = cli.command.clone().unwrap_or(CliCommand::Run);
    match command {
        CliCommand::Run => run_runtime(cli).await,
        CliCommand::Status(args) => run_status_command(args).await,
        CliCommand::Health(args) => run_health_command(args).await,
        CliCommand::Doctor(args) => run_doctor(&cli.config, args),
        CliCommand::Security(args) => run_security_command(&cli.config, args).await,
        CliCommand::Gateway(args) => run_gateway_command(cli, args).await,
        CliCommand::Tools(args) => run_tools_command(args).await,
        CliCommand::Agent(args) => run_agent_command(args).await,
        CliCommand::Message(args) => run_message_command(args).await,
        CliCommand::Nodes(args) => run_nodes_command(args).await,
        CliCommand::Sessions(args) => run_sessions_command(args).await,
    }
}

async fn run_runtime(cli: Cli) -> Result<()> {
    let mut cfg = Config::load(&cli.config)?;
    cfg.apply_cli_overrides(
        cli.gateway_url.as_deref(),
        cli.gateway_token.as_deref(),
        cli.audit_only,
    );

    let runtime = runtime::AgentRuntime::new(cfg, Some(cli.config.clone())).await?;
    runtime.run().await
}

async fn run_status_command(args: StatusArgs) -> Result<()> {
    let dispatcher = RpcDispatcher::new();
    let payload = dispatch_rpc(&dispatcher, "status", json!({})).await?;
    if args.json {
        print_json_value(&payload);
    } else {
        let runtime_name = payload
            .pointer("/runtime/name")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let runtime_version = payload
            .pointer("/runtime/version")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let uptime_ms = payload
            .pointer("/runtime/uptimeMs")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let sessions = payload
            .pointer("/sessions/totalSessions")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        println!(
            "status: runtime={runtime_name}@{runtime_version} uptime_ms={uptime_ms} total_sessions={sessions}"
        );
    }
    Ok(())
}

async fn run_health_command(args: HealthArgs) -> Result<()> {
    let dispatcher = RpcDispatcher::new();
    let payload = dispatch_rpc(&dispatcher, "health", json!({})).await?;
    if args.json {
        print_json_value(&payload);
    } else {
        let service = payload
            .get("service")
            .and_then(Value::as_str)
            .unwrap_or("openclaw-agent-rs");
        let ok = payload.get("ok").and_then(Value::as_bool).unwrap_or(false);
        let uptime_ms = payload.get("uptimeMs").and_then(Value::as_u64).unwrap_or(0);
        println!("health: service={service} ok={ok} uptime_ms={uptime_ms}");
    }
    Ok(())
}

fn run_doctor(config_path: &Path, args: DoctorArgs) -> Result<()> {
    let _ = args.non_interactive;
    let config_result = Config::load(config_path).map_err(|err| err.to_string());
    let report = build_doctor_report(config_result, config_path, command_available("docker"));
    print_doctor_report(&report, args.json);
    if report.ok {
        return Ok(());
    }
    Err(anyhow!("doctor reported blocking issues"))
}

async fn run_security_command(config_path: &Path, args: SecurityArgs) -> Result<()> {
    match args
        .command
        .unwrap_or(SecuritySubcommand::Audit(SecurityAuditArgs::default()))
    {
        SecuritySubcommand::Audit(audit) => {
            let run = security_audit::run_security_audit(
                config_path,
                security_audit::SecurityAuditOptions {
                    deep: audit.deep,
                    fix: audit.fix,
                },
            )
            .await;

            if audit.json {
                if audit.fix {
                    let value = serde_json::to_value(&run).unwrap_or_else(|_| {
                        json!({
                            "report": {
                                "ts": 0,
                                "summary": { "critical": 0, "warn": 0, "info": 0 },
                                "findings": []
                            }
                        })
                    });
                    print_json_value(&value);
                } else {
                    let value = serde_json::to_value(&run.report).unwrap_or_else(|_| {
                        json!({
                            "ts": 0,
                            "summary": { "critical": 0, "warn": 0, "info": 0 },
                            "findings": []
                        })
                    });
                    print_json_value(&value);
                }
                return Ok(());
            }

            print_security_audit_report(&run.report, run.fix.as_ref());
            Ok(())
        }
    }
}

async fn run_gateway_command(cli: Cli, args: GatewayArgs) -> Result<()> {
    match args.command.unwrap_or(GatewaySubcommand::Run) {
        GatewaySubcommand::Run => run_runtime(cli).await,
        GatewaySubcommand::Status => run_status_command(StatusArgs { json: args.json }).await,
        GatewaySubcommand::Health => run_health_command(HealthArgs { json: args.json }).await,
        GatewaySubcommand::Methods => {
            let methods = gateway::supported_rpc_methods();
            if args.json {
                print_json_value(&json!({
                    "count": methods.len(),
                    "methods": methods
                }));
            } else {
                println!("gateway methods: {}", methods.len());
                for method in methods {
                    println!("{method}");
                }
            }
            Ok(())
        }
        GatewaySubcommand::Call(call) => {
            let method = call.method.trim();
            if method.is_empty() {
                return Err(anyhow!("gateway call requires --method"));
            }
            let params: Value = serde_json::from_str(&call.params)
                .map_err(|err| anyhow!("invalid --params JSON: {err}"))?;
            let payload = if call.live_service {
                dispatch_gateway_rpc_live(&cli, method, params).await?
            } else {
                let dispatcher = RpcDispatcher::new();
                dispatch_rpc(&dispatcher, method, params).await?
            };
            if args.json || call.json {
                print_json_value(&payload);
            } else {
                println!("gateway call: method={method}");
                print_json_value(&payload);
            }
            Ok(())
        }
    }
}

type CliGatewayWsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn dispatch_gateway_rpc_live(cli: &Cli, method: &str, params: Value) -> Result<Value> {
    let mut cfg = Config::load(&cli.config)?;
    cfg.apply_cli_overrides(
        cli.gateway_url.as_deref(),
        cli.gateway_token.as_deref(),
        cli.audit_only,
    );
    let gateway_url = cfg.gateway.url.trim();
    if gateway_url.is_empty() {
        return Err(anyhow!("gateway.url is empty"));
    }

    let (mut ws, _) = connect_async(gateway_url)
        .await
        .map_err(|err| anyhow!("gateway websocket connect failed: {err}"))?;

    let handshake_timeout = Duration::from_millis(cfg.gateway.server.handshake_timeout_ms.max(500));
    let challenge =
        gateway_ws_read_next_json(&mut ws, handshake_timeout, "connect.challenge").await?;
    if challenge.get("event").and_then(Value::as_str) != Some("connect.challenge") {
        return Err(anyhow!(
            "invalid gateway handshake: expected connect.challenge event"
        ));
    }

    let connect_id = next_cli_request_id("connect");
    let mut connect_request = json!({
        "type": "req",
        "id": connect_id,
        "method": "connect",
        "params": {
            "role": "operator",
            "scopes": ["operator.admin"],
            "client": {
                "id": "openclaw-agent-rs.cli",
                "version": env!("CARGO_PKG_VERSION"),
                "mode": "cli"
            }
        }
    });
    if let Some(token) = cfg
        .gateway
        .token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        connect_request["params"]["auth"]["token"] = Value::String(token.to_owned());
    }
    if let Some(password) = cfg
        .gateway
        .password
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        connect_request["params"]["auth"]["password"] = Value::String(password.to_owned());
    }
    ws.send(Message::Text(connect_request.to_string()))
        .await
        .map_err(|err| anyhow!("gateway connect request send failed: {err}"))?;
    let connect_response =
        gateway_ws_wait_for_response(&mut ws, &connect_id, handshake_timeout).await?;
    if !connect_response
        .get("ok")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let code = connect_response
            .pointer("/error/code")
            .and_then(Value::as_i64)
            .unwrap_or(-1);
        let message = connect_response
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("gateway connect rejected");
        return Err(anyhow!(
            "live gateway connect failed: code={code} message={message}"
        ));
    }

    let request_id = next_cli_request_id(method);
    let request = json!({
        "type": "req",
        "id": request_id,
        "method": method,
        "params": params
    });
    ws.send(Message::Text(request.to_string()))
        .await
        .map_err(|err| anyhow!("gateway request send failed ({method}): {err}"))?;
    let response =
        gateway_ws_wait_for_response(&mut ws, &request_id, Duration::from_secs(30)).await?;
    if !response.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        let code = response
            .pointer("/error/code")
            .and_then(Value::as_i64)
            .unwrap_or(-1);
        let message = response
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("gateway request failed");
        if let Some(details) = response.pointer("/error/details") {
            return Err(anyhow!(
                "live gateway rpc {method} failed: code={code} message={message} details={details}"
            ));
        }
        return Err(anyhow!(
            "live gateway rpc {method} failed: code={code} message={message}"
        ));
    }
    Ok(response.get("result").cloned().unwrap_or(Value::Null))
}

async fn gateway_ws_wait_for_response(
    ws: &mut CliGatewayWsStream,
    response_id: &str,
    timeout: Duration,
) -> Result<Value> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err(anyhow!(
                "gateway request timed out waiting for id {response_id}"
            ));
        }
        let remaining = deadline.saturating_duration_since(now);
        let frame =
            gateway_ws_read_next_json(ws, remaining, &format!("response id {response_id}")).await?;
        if frame.get("id").and_then(Value::as_str) == Some(response_id) {
            return Ok(frame);
        }
    }
}

async fn gateway_ws_read_next_json(
    ws: &mut CliGatewayWsStream,
    timeout: Duration,
    context: &str,
) -> Result<Value> {
    let result = tokio::time::timeout(timeout, async {
        loop {
            let Some(message) = ws.next().await else {
                return Err(anyhow!("gateway socket closed while waiting for {context}"));
            };
            let message = message.map_err(|err| anyhow!("gateway websocket read failed: {err}"))?;
            match message {
                Message::Text(text) => {
                    let parsed = serde_json::from_str::<Value>(text.as_ref())
                        .map_err(|err| anyhow!("gateway frame JSON parse failed: {err}"))?;
                    return Ok(parsed);
                }
                Message::Ping(payload) => {
                    ws.send(Message::Pong(payload))
                        .await
                        .map_err(|err| anyhow!("gateway websocket pong failed: {err}"))?;
                }
                Message::Close(frame) => {
                    let reason = frame
                        .as_ref()
                        .map(|value| value.reason.to_string())
                        .unwrap_or_else(|| "close".to_owned());
                    return Err(anyhow!("gateway socket closed: {reason}"));
                }
                Message::Binary(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    })
    .await;
    match result {
        Ok(value) => value,
        Err(_) => Err(anyhow!(
            "gateway timed out waiting for {context} after {}ms",
            timeout.as_millis()
        )),
    }
}

async fn run_tools_command(args: ToolsArgs) -> Result<()> {
    match args
        .command
        .unwrap_or(ToolsSubcommand::Catalog(ToolsCatalogCommandArgs::default()))
    {
        ToolsSubcommand::Catalog(catalog) => run_tools_catalog_command(args.json, catalog).await,
    }
}

async fn run_tools_catalog_command(global_json: bool, args: ToolsCatalogCommandArgs) -> Result<()> {
    let dispatcher = RpcDispatcher::new();
    let mut params = json!({});
    if let Some(agent_id) = args.agent_id {
        params["agentId"] = json!(agent_id);
    }
    if let Some(include_plugins) = args.include_plugins {
        params["includePlugins"] = json!(include_plugins);
    }

    let payload = dispatch_rpc(&dispatcher, "tools.catalog", params).await?;
    if global_json || args.json {
        print_json_value(&payload);
    } else {
        let agent_id = payload
            .get("agentId")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let profile_count = payload
            .get("profiles")
            .and_then(Value::as_array)
            .map(|items| items.len())
            .unwrap_or(0);
        let groups = payload
            .get("groups")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let group_count = groups.len();
        let tool_count = groups
            .iter()
            .map(|group| {
                group
                    .get("tools")
                    .and_then(Value::as_array)
                    .map(|items| items.len())
                    .unwrap_or(0)
            })
            .sum::<usize>();
        println!(
            "tools catalog: agent_id={agent_id} profiles={profile_count} groups={group_count} tools={tool_count}"
        );
    }
    Ok(())
}

async fn run_agent_command(args: AgentArgs) -> Result<()> {
    let dispatcher = RpcDispatcher::new();
    let run_id = args
        .idempotency_key
        .unwrap_or_else(|| format!("cli-agent-{}", now_ms()));
    let mut params = json!({
        "message": args.message,
        "sessionKey": args.session_key,
        "idempotencyKey": run_id
    });
    if let Some(agent_id) = args.agent_id {
        params["agentId"] = json!(agent_id);
    }
    if let Some(channel) = args.channel {
        params["channel"] = json!(channel);
    }
    if let Some(to) = args.to {
        params["to"] = json!(to);
    }
    if let Some(account_id) = args.account_id {
        params["accountId"] = json!(account_id);
    }
    if let Some(thread_id) = args.thread_id {
        params["threadId"] = json!(thread_id);
    }
    if let Some(thinking) = args.thinking {
        params["thinking"] = json!(thinking);
    }

    let run_payload = dispatch_rpc(&dispatcher, "agent", params).await?;
    let payload = if args.wait {
        let wait_run_id = run_payload
            .get("runId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let wait_payload = dispatch_rpc(
            &dispatcher,
            "agent.wait",
            json!({
                "runId": wait_run_id,
                "timeoutMs": args.timeout_ms
            }),
        )
        .await?;
        json!({
            "run": run_payload,
            "wait": wait_payload
        })
    } else {
        run_payload
    };

    if args.json {
        print_json_value(&payload);
    } else {
        let run = payload.get("run").unwrap_or(&payload);
        let run_id = run
            .get("runId")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let run_status = run
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if let Some(wait) = payload.get("wait") {
            let wait_status = wait
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            println!("agent run: run_id={run_id} status={run_status} wait_status={wait_status}");
        } else {
            println!("agent run: run_id={run_id} status={run_status}");
        }
    }
    Ok(())
}

async fn run_message_command(args: MessageArgs) -> Result<()> {
    match args.command {
        MessageSubcommand::Send(send) => run_message_send_command(send).await,
    }
}

async fn run_message_send_command(args: MessageSendArgs) -> Result<()> {
    let dispatcher = RpcDispatcher::new();
    let idempotency_key = args
        .idempotency_key
        .unwrap_or_else(|| format!("cli-send-{}", now_ms()));
    let mut params = json!({
        "to": args.to,
        "idempotencyKey": idempotency_key
    });
    if let Some(message) = args.message {
        params["message"] = json!(message);
    }
    if !args.media_urls.is_empty() {
        params["mediaUrls"] = json!(args.media_urls);
    }
    if let Some(channel) = args.channel {
        params["channel"] = json!(channel);
    }
    if let Some(account_id) = args.account_id {
        params["accountId"] = json!(account_id);
    }
    if let Some(thread_id) = args.thread_id {
        params["threadId"] = json!(thread_id);
    }
    if let Some(session_key) = args.session_key {
        params["sessionKey"] = json!(session_key);
    }

    let payload = dispatch_rpc(&dispatcher, "send", params).await?;
    if args.json {
        print_json_value(&payload);
    } else {
        let run_id = payload
            .get("runId")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let message_id = payload
            .get("messageId")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let channel = payload
            .get("channel")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        println!("message sent: run_id={run_id} message_id={message_id} channel={channel}");
    }
    Ok(())
}

async fn run_nodes_command(args: NodesArgs) -> Result<()> {
    match args.command.unwrap_or(NodesSubcommand::List) {
        NodesSubcommand::List => {
            let dispatcher = RpcDispatcher::new();
            let payload = dispatch_rpc(&dispatcher, "node.list", json!({})).await?;
            if args.json {
                print_json_value(&payload);
            } else {
                let count = payload
                    .get("nodes")
                    .and_then(Value::as_array)
                    .map(|items| items.len())
                    .unwrap_or(0);
                println!("nodes list: count={count}");
            }
            Ok(())
        }
    }
}

async fn run_sessions_command(args: SessionsArgs) -> Result<()> {
    match args.command {
        SessionsSubcommand::List(list) => {
            let dispatcher = RpcDispatcher::new();
            let mut params = json!({});
            if let Some(limit) = list.limit {
                params["limit"] = json!(limit);
            }
            if let Some(agent_id) = list.agent_id {
                params["agentId"] = json!(agent_id);
            }
            if let Some(channel) = list.channel {
                params["channel"] = json!(channel);
            }
            if let Some(search) = list.search {
                params["search"] = json!(search);
            }
            if list.include_derived_titles {
                params["includeDerivedTitles"] = json!(true);
            }
            if list.include_last_message {
                params["includeLastMessage"] = json!(true);
            }
            let payload = dispatch_rpc(&dispatcher, "sessions.list", params).await?;
            if list.json {
                print_json_value(&payload);
            } else {
                let count = payload.get("count").and_then(Value::as_u64).unwrap_or(0);
                println!("sessions list: count={count}");
            }
            Ok(())
        }
        SessionsSubcommand::Status(status) => {
            let dispatcher = RpcDispatcher::new();
            let payload = dispatch_rpc(
                &dispatcher,
                "session.status",
                json!({
                    "sessionKey": status.session_key
                }),
            )
            .await?;
            if status.json {
                print_json_value(&payload);
            } else {
                let session_id = payload
                    .pointer("/session/sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let key = payload
                    .pointer("/session/key")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                println!("session status: key={key} session_id={session_id}");
            }
            Ok(())
        }
    }
}

async fn dispatch_rpc(dispatcher: &RpcDispatcher, method: &str, params: Value) -> Result<Value> {
    let request = RpcRequestFrame {
        id: next_cli_request_id(method),
        method: method.to_owned(),
        params,
    };
    match dispatcher.handle_request(&request).await {
        RpcDispatchOutcome::Handled(payload) => Ok(payload),
        RpcDispatchOutcome::NotHandled => Err(anyhow!("rpc method not handled: {method}")),
        RpcDispatchOutcome::Error {
            code,
            message,
            details,
        } => {
            if let Some(details) = details {
                Err(anyhow!(
                    "rpc method {method} failed: code={code} message={message} details={}",
                    details
                ))
            } else {
                Err(anyhow!(
                    "rpc method {method} failed: code={code} message={message}"
                ))
            }
        }
    }
}

fn next_cli_request_id(method: &str) -> String {
    let method_label = method.replace('.', "-");
    format!("cli-{method_label}-{}", now_ms())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn print_json_value(value: &Value) {
    let rendered = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    println!("{rendered}");
}

fn build_doctor_report(
    config_result: std::result::Result<Config, String>,
    config_path: &Path,
    docker_available: bool,
) -> DoctorReport {
    let mut checks = Vec::new();
    let mut config = None;

    match config_result {
        Ok(cfg) => {
            checks.push(DoctorCheck {
                id: "config.load".to_owned(),
                status: "pass".to_owned(),
                message: format!("loaded {}", config_path.display()),
                detail: None,
            });
            config = Some(cfg);
        }
        Err(err) => {
            checks.push(DoctorCheck {
                id: "config.load".to_owned(),
                status: "fail".to_owned(),
                message: format!("failed to load {}", config_path.display()),
                detail: Some(err),
            });
        }
    }

    if let Some(cfg) = config.as_ref() {
        checks.push(DoctorCheck {
            id: "gateway.runtime_mode".to_owned(),
            status: "pass".to_owned(),
            message: format!("{:?}", cfg.gateway.runtime_mode),
            detail: None,
        });

        let (auth_ok, auth_detail) = match cfg.gateway.server.auth_mode {
            GatewayAuthMode::Token => (
                cfg.gateway
                    .token
                    .as_deref()
                    .map(str::trim)
                    .map(|value| !value.is_empty())
                    .unwrap_or(false),
                "token required",
            ),
            GatewayAuthMode::Password => (
                cfg.gateway
                    .password
                    .as_deref()
                    .map(str::trim)
                    .map(|value| !value.is_empty())
                    .unwrap_or(false),
                "password required",
            ),
            GatewayAuthMode::Auto | GatewayAuthMode::None => (true, "not required"),
        };
        checks.push(DoctorCheck {
            id: "gateway.auth_secret".to_owned(),
            status: if auth_ok { "pass" } else { "fail" }.to_owned(),
            message: format!("{:?}", cfg.gateway.server.auth_mode),
            detail: Some(auth_detail.to_owned()),
        });

        let state_path = cfg
            .runtime
            .session_state_path
            .to_string_lossy()
            .to_ascii_lowercase();
        let sqlite_selected = state_path.ends_with(".db")
            || state_path.ends_with(".sqlite")
            || state_path.ends_with(".sqlite3");
        let sqlite_enabled = cfg!(feature = "sqlite-state");
        checks.push(DoctorCheck {
            id: "runtime.sqlite_state".to_owned(),
            status: if sqlite_selected && !sqlite_enabled {
                "warn"
            } else {
                "pass"
            }
            .to_owned(),
            message: if sqlite_selected {
                "sqlite-backed session state requested".to_owned()
            } else {
                "json-backed session state requested".to_owned()
            },
            detail: Some(format!("feature sqlite-state enabled={sqlite_enabled}")),
        });

        let wasm_mode = match cfg.security.wasm.tool_runtime_mode {
            ToolRuntimeWasmMode::InspectionStub => "inspection_stub",
            ToolRuntimeWasmMode::WasmSandbox => "wasm_sandbox",
        };
        checks.push(DoctorCheck {
            id: "security.wasm_runtime_mode".to_owned(),
            status: "pass".to_owned(),
            message: wasm_mode.to_owned(),
            detail: None,
        });
        checks.push(DoctorCheck {
            id: "security.wasm_enabled".to_owned(),
            status: if cfg.security.tool_runtime_policy.wasm.enabled {
                "pass"
            } else {
                "warn"
            }
            .to_owned(),
            message: if cfg.security.tool_runtime_policy.wasm.enabled {
                "wasm tool runtime enabled".to_owned()
            } else {
                "wasm tool runtime disabled".to_owned()
            },
            detail: Some(
                "set security.tool_runtime_policy.wasm.enabled=true for live wasm execution"
                    .to_owned(),
            ),
        });
        let wit_root = cfg.security.tool_runtime_policy.wasm.wit_root.clone();
        checks.push(DoctorCheck {
            id: "security.wasm_wit_root".to_owned(),
            status: if wit_root.exists() { "pass" } else { "warn" }.to_owned(),
            message: wit_root.display().to_string(),
            detail: Some("dynamic WIT tool registry source root".to_owned()),
        });
        let module_root = cfg.security.tool_runtime_policy.wasm.module_root.clone();
        checks.push(DoctorCheck {
            id: "security.wasm_module_root".to_owned(),
            status: if module_root.exists() { "pass" } else { "warn" }.to_owned(),
            message: module_root.display().to_string(),
            detail: Some("wasm module storage root".to_owned()),
        });
    }

    checks.push(DoctorCheck {
        id: "docker.binary".to_owned(),
        status: if docker_available { "pass" } else { "warn" }.to_owned(),
        message: if docker_available {
            "docker is available".to_owned()
        } else {
            "docker is not available".to_owned()
        },
        detail: Some("required for full parity stack tests".to_owned()),
    });
    let wasmtime_available = command_available("wasmtime");
    checks.push(DoctorCheck {
        id: "wasmtime.binary".to_owned(),
        status: if wasmtime_available { "pass" } else { "warn" }.to_owned(),
        message: if wasmtime_available {
            "wasmtime CLI is available".to_owned()
        } else {
            "wasmtime CLI is not available".to_owned()
        },
        detail: Some("optional but useful for standalone wasm inspection".to_owned()),
    });

    let ok = checks.iter().all(|check| check.status != "fail");
    DoctorReport { ok, checks }
}

fn print_doctor_report(report: &DoctorReport, json_output: bool) {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(report)
                .unwrap_or_else(|_| "{\"ok\":false,\"checks\":[]}".to_owned())
        );
        return;
    }

    println!("doctor: {}", if report.ok { "ok" } else { "issues" });
    for check in &report.checks {
        let detail = check
            .detail
            .as_deref()
            .map(|value| format!(" ({value})"))
            .unwrap_or_default();
        println!(
            "[{}] {}: {}{}",
            check.status.to_uppercase(),
            check.id,
            check.message,
            detail
        );
    }
}

fn print_security_audit_report(
    report: &security_audit::SecurityAuditReport,
    fix: Option<&security_audit::SecurityFixResult>,
) {
    println!(
        "security audit: critical={} warn={} info={}",
        report.summary.critical, report.summary.warn, report.summary.info
    );

    if let Some(deep) = report.deep.as_ref() {
        if let Some(gateway) = deep.gateway.as_ref() {
            let url = gateway.url.as_deref().unwrap_or("n/a");
            let status = if gateway.ok { "ok" } else { "failed" };
            let detail = gateway
                .error
                .as_deref()
                .map(|value| format!(" ({value})"))
                .unwrap_or_default();
            println!("deep.gateway: {status} url={url}{detail}");
        }
    }

    if let Some(fix) = fix {
        println!("fix: ok={} changes={}", fix.ok, fix.changes.len());
        for change in &fix.changes {
            println!("  change: {change}");
        }
        for action in &fix.actions {
            let mut suffix = String::new();
            if let Some(mode) = action.mode.as_deref() {
                suffix.push_str(&format!(" mode={mode}"));
            }
            if let Some(skipped) = action.skipped.as_deref() {
                suffix.push_str(&format!(" skipped={skipped}"));
            }
            if let Some(error) = action.error.as_deref() {
                suffix.push_str(&format!(" error={error}"));
            }
            println!(
                "  action: kind={} target={} ok={}{}",
                action.kind, action.target, action.ok, suffix
            );
        }
        for error in &fix.errors {
            println!("  error: {error}");
        }
    }

    for finding in &report.findings {
        let severity = match finding.severity {
            security_audit::SecurityAuditSeverity::Critical => "CRITICAL",
            security_audit::SecurityAuditSeverity::Warn => "WARN",
            security_audit::SecurityAuditSeverity::Info => "INFO",
        };
        println!(
            "[{severity}] {}: {} - {}",
            finding.check_id, finding.title, finding.detail
        );
        if let Some(remediation) = finding.remediation.as_deref() {
            println!("  fix: {remediation}");
        }
    }
}

fn command_available(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn init_logging(filter: &str) -> Result<()> {
    let env = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter));
    tracing_subscriber::fmt()
        .with_env_filter(env)
        .with_target(false)
        .init();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parses_doctor_command_and_flags() {
        let cli = Cli::parse_from(["openclaw-agent-rs", "doctor", "--non-interactive", "--json"]);
        match cli.command {
            Some(CliCommand::Doctor(args)) => {
                assert!(args.non_interactive);
                assert!(args.json);
            }
            _ => panic!("expected doctor command"),
        }
    }

    #[test]
    fn cli_parses_security_audit_command_and_flags() {
        let cli = Cli::parse_from([
            "openclaw-agent-rs",
            "security",
            "audit",
            "--deep",
            "--fix",
            "--json",
        ]);
        match cli.command {
            Some(CliCommand::Security(SecurityArgs {
                command: Some(SecuritySubcommand::Audit(args)),
            })) => {
                assert!(args.deep);
                assert!(args.fix);
                assert!(args.json);
            }
            _ => panic!("expected security audit command"),
        }
    }

    #[test]
    fn cli_parses_gateway_status_command_and_json_flag() {
        let cli = Cli::parse_from(["openclaw-agent-rs", "gateway", "--json", "status"]);
        match cli.command {
            Some(CliCommand::Gateway(args)) => {
                assert!(args.json);
                assert!(matches!(args.command, Some(GatewaySubcommand::Status)));
            }
            _ => panic!("expected gateway command"),
        }
    }

    #[test]
    fn cli_parses_gateway_call_command_with_params() {
        let cli = Cli::parse_from([
            "openclaw-agent-rs",
            "gateway",
            "call",
            "--method",
            "tools.catalog",
            "--params",
            "{\"includePlugins\":false}",
            "--json",
        ]);
        match cli.command {
            Some(CliCommand::Gateway(args)) => match args.command {
                Some(GatewaySubcommand::Call(call)) => {
                    assert_eq!(call.method, "tools.catalog");
                    assert_eq!(call.params, "{\"includePlugins\":false}");
                    assert!(!call.live_service);
                    assert!(call.json);
                }
                _ => panic!("expected gateway call command"),
            },
            _ => panic!("expected gateway command"),
        }
    }

    #[test]
    fn cli_parses_gateway_call_command_with_live_service_flag() {
        let cli = Cli::parse_from([
            "openclaw-agent-rs",
            "gateway",
            "call",
            "--method",
            "auth.oauth.wait",
            "--live-service",
            "--json",
        ]);
        match cli.command {
            Some(CliCommand::Gateway(args)) => match args.command {
                Some(GatewaySubcommand::Call(call)) => {
                    assert_eq!(call.method, "auth.oauth.wait");
                    assert_eq!(call.params, "{}");
                    assert!(call.live_service);
                    assert!(call.json);
                }
                _ => panic!("expected gateway call command"),
            },
            _ => panic!("expected gateway command"),
        }
    }

    #[test]
    fn cli_parses_tools_catalog_command_with_agent_and_plugin_flag() {
        let cli = Cli::parse_from([
            "openclaw-agent-rs",
            "tools",
            "--json",
            "catalog",
            "--agent-id",
            "main",
            "--include-plugins",
            "false",
        ]);
        match cli.command {
            Some(CliCommand::Tools(args)) => {
                assert!(args.json);
                match args.command {
                    Some(ToolsSubcommand::Catalog(catalog)) => {
                        assert_eq!(catalog.agent_id.as_deref(), Some("main"));
                        assert_eq!(catalog.include_plugins, Some(false));
                    }
                    _ => panic!("expected tools catalog command"),
                }
            }
            _ => panic!("expected tools command"),
        }
    }

    #[test]
    fn cli_parses_message_send_command() {
        let cli = Cli::parse_from([
            "openclaw-agent-rs",
            "message",
            "send",
            "--to",
            "+1234567890",
            "--message",
            "hi",
            "--channel",
            "telegram",
            "--json",
        ]);
        match cli.command {
            Some(CliCommand::Message(MessageArgs {
                command: MessageSubcommand::Send(args),
            })) => {
                assert_eq!(args.to, "+1234567890");
                assert_eq!(args.message.as_deref(), Some("hi"));
                assert_eq!(args.channel.as_deref(), Some("telegram"));
                assert!(args.json);
            }
            _ => panic!("expected message send command"),
        }
    }

    #[tokio::test]
    async fn cli_dispatch_rpc_status_returns_runtime_payload() {
        let dispatcher = RpcDispatcher::new();
        let payload = dispatch_rpc(&dispatcher, "status", json!({}))
            .await
            .expect("status rpc should succeed");
        assert_eq!(
            payload.pointer("/runtime/name").and_then(Value::as_str),
            Some("openclaw-agent-rs")
        );
        assert!(
            payload
                .pointer("/rpc/count")
                .and_then(Value::as_u64)
                .unwrap_or_default()
                > 0
        );
    }

    #[tokio::test]
    async fn cli_dispatch_rpc_send_returns_message_identifiers() {
        let dispatcher = RpcDispatcher::new();
        let payload = dispatch_rpc(
            &dispatcher,
            "send",
            json!({
                "to": "+15551234567",
                "message": "cp7 message parity",
                "channel": "telegram",
                "idempotencyKey": "cp7-send-1"
            }),
        )
        .await
        .expect("send rpc should succeed");
        assert_eq!(
            payload.get("runId").and_then(Value::as_str),
            Some("cp7-send-1")
        );
        assert!(payload
            .get("messageId")
            .and_then(Value::as_str)
            .map(|value| !value.is_empty())
            .unwrap_or(false));
    }

    #[tokio::test]
    async fn cli_dispatch_rpc_sessions_list_returns_count_field() {
        let dispatcher = RpcDispatcher::new();
        let payload = dispatch_rpc(
            &dispatcher,
            "sessions.list",
            json!({
                "limit": 10,
                "includeLastMessage": true
            }),
        )
        .await
        .expect("sessions.list rpc should succeed");
        assert!(payload.get("count").and_then(Value::as_u64).is_some());
        assert!(payload.get("sessions").and_then(Value::as_array).is_some());
    }

    #[test]
    fn doctor_report_marks_config_load_failure_as_blocking() {
        let report = build_doctor_report(
            Err("invalid config".to_owned()),
            Path::new("openclaw-rs.toml"),
            false,
        );
        assert!(!report.ok);
        assert!(report
            .checks
            .iter()
            .any(|check| check.id == "config.load" && check.status == "fail"));
    }

    #[test]
    fn doctor_report_warns_when_docker_is_unavailable() {
        let report =
            build_doctor_report(Ok(Config::default()), Path::new("openclaw-rs.toml"), false);
        assert!(report.ok);
        assert!(report
            .checks
            .iter()
            .any(|check| check.id == "docker.binary" && check.status == "warn"));
    }

    #[test]
    fn doctor_report_includes_wasm_checks() {
        let report =
            build_doctor_report(Ok(Config::default()), Path::new("openclaw-rs.toml"), true);
        assert!(report
            .checks
            .iter()
            .any(|check| check.id == "security.wasm_runtime_mode"));
        assert!(report
            .checks
            .iter()
            .any(|check| check.id == "security.wasm_wit_root"));
    }
}
