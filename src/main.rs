mod bridge;
mod channels;
mod config;
mod gateway;
mod gateway_server;
mod memory;
mod protocol;
mod runtime;
mod scheduler;
mod security;
mod session_key;
mod state;
mod tool_runtime;
mod types;

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Result};
use clap::{Args, Parser, Subcommand};
use config::{Config, GatewayAuthMode};
use serde::Serialize;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
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
    /// Run non-interactive diagnostics for operator parity checks.
    Doctor(DoctorArgs),
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
        CliCommand::Doctor(args) => run_doctor(&cli.config, args),
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
}
