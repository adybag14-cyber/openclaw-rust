use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::net::TcpStream;
use tokio::time::timeout;
use url::Url;

use crate::config::{Config, GatewayAuthMode, GatewayRuntimeMode, GroupActivationMode};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SecurityAuditSeverity {
    Info,
    Warn,
    Critical,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecurityAuditFinding {
    #[serde(rename = "checkId")]
    pub check_id: String,
    pub severity: SecurityAuditSeverity,
    pub title: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SecurityAuditSummary {
    pub critical: usize,
    pub warn: usize,
    pub info: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecurityAuditDeepGateway {
    pub attempted: bool,
    pub url: Option<String>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecurityAuditDeepReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway: Option<SecurityAuditDeepGateway>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecurityAuditReport {
    pub ts: u64,
    pub summary: SecurityAuditSummary,
    pub findings: Vec<SecurityAuditFinding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deep: Option<SecurityAuditDeepReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecurityFixAction {
    pub kind: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecurityFixResult {
    pub ok: bool,
    pub changes: Vec<String>,
    pub actions: Vec<SecurityFixAction>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecurityAuditRun {
    pub report: SecurityAuditReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<SecurityFixResult>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SecurityAuditOptions {
    pub deep: bool,
    pub fix: bool,
}

impl SecurityAuditSummary {
    fn from_findings(findings: &[SecurityAuditFinding]) -> Self {
        let mut summary = Self::default();
        for finding in findings {
            match finding.severity {
                SecurityAuditSeverity::Critical => summary.critical += 1,
                SecurityAuditSeverity::Warn => summary.warn += 1,
                SecurityAuditSeverity::Info => summary.info += 1,
            }
        }
        summary
    }
}

pub async fn run_security_audit(
    config_path: &Path,
    options: SecurityAuditOptions,
) -> SecurityAuditRun {
    let fix = if options.fix {
        match Config::load(config_path) {
            Ok(cfg) => Some(apply_security_fixes(config_path, &cfg)),
            Err(err) => Some(SecurityFixResult {
                ok: false,
                changes: Vec::new(),
                actions: Vec::new(),
                errors: vec![format!("failed to load config for --fix: {err}")],
            }),
        }
    } else {
        None
    };

    let mut findings = Vec::new();
    let cfg = match Config::load(config_path) {
        Ok(cfg) => Some(cfg),
        Err(err) => {
            findings.push(SecurityAuditFinding {
                check_id: "config.load".to_owned(),
                severity: SecurityAuditSeverity::Critical,
                title: "Config failed to load".to_owned(),
                detail: format!("failed to load {}: {err}", config_path.display()),
                remediation: Some(
                    "fix config syntax/values, then rerun `openclaw-agent-rs security audit`"
                        .to_owned(),
                ),
            });
            None
        }
    };

    if let Some(cfg) = cfg.as_ref() {
        collect_config_findings(cfg, config_path, &mut findings);
    }

    let deep = if options.deep {
        let gateway = if let Some(cfg) = cfg.as_ref() {
            let probe = deep_probe_gateway(cfg).await;
            if !probe.ok {
                findings.push(SecurityAuditFinding {
                    check_id: "gateway.deep_probe".to_owned(),
                    severity: SecurityAuditSeverity::Warn,
                    title: "Deep gateway probe failed".to_owned(),
                    detail: probe
                        .error
                        .clone()
                        .unwrap_or_else(|| "gateway probe failed".to_owned()),
                    remediation: Some(
                        "ensure the gateway URL is reachable, then rerun `openclaw-agent-rs security audit --deep`"
                            .to_owned(),
                    ),
                });
            }
            Some(probe)
        } else {
            Some(SecurityAuditDeepGateway {
                attempted: false,
                url: None,
                ok: false,
                error: Some("deep probe skipped because config failed to load".to_owned()),
            })
        };
        Some(SecurityAuditDeepReport { gateway })
    } else {
        None
    };

    let summary = SecurityAuditSummary::from_findings(&findings);
    let report = SecurityAuditReport {
        ts: now_ms(),
        summary,
        findings,
        deep,
    };

    SecurityAuditRun { report, fix }
}

fn collect_config_findings(
    cfg: &Config,
    config_path: &Path,
    findings: &mut Vec<SecurityAuditFinding>,
) {
    let auth_mode = cfg.gateway.server.auth_mode;
    let has_token = cfg
        .gateway
        .token
        .as_deref()
        .map(str::trim)
        .map(|value| !value.is_empty())
        .unwrap_or(false);
    let has_password = cfg
        .gateway
        .password
        .as_deref()
        .map(str::trim)
        .map(|value| !value.is_empty())
        .unwrap_or(false);

    if auth_mode == GatewayAuthMode::None {
        findings.push(SecurityAuditFinding {
            check_id: "gateway.auth.none".to_owned(),
            severity: SecurityAuditSeverity::Critical,
            title: "Gateway auth is disabled".to_owned(),
            detail: "gateway.server.auth_mode is set to `none`; control APIs can be exposed without a shared secret.".to_owned(),
            remediation: Some(
                "set `gateway.server.auth_mode` to `token` or `password`, then configure `gateway.token`/`gateway.password`"
                    .to_owned(),
            ),
        });
    } else if auth_mode == GatewayAuthMode::Auto && !has_token && !has_password {
        findings.push(SecurityAuditFinding {
            check_id: "gateway.auth.auto_unset".to_owned(),
            severity: SecurityAuditSeverity::Warn,
            title: "Gateway auth is auto with no explicit secret".to_owned(),
            detail: "gateway.server.auth_mode is `auto`, but no token/password is configured.".to_owned(),
            remediation: Some(
                "set `gateway.token` (or `gateway.password`) explicitly for predictable auth posture"
                    .to_owned(),
            ),
        });
    }

    if cfg.gateway.runtime_mode == GatewayRuntimeMode::StandaloneServer {
        let bind = cfg.gateway.server.bind.trim();
        if !is_loopback_bind(bind) {
            findings.push(SecurityAuditFinding {
                check_id: "gateway.bind.public".to_owned(),
                severity: if auth_mode == GatewayAuthMode::None {
                    SecurityAuditSeverity::Critical
                } else {
                    SecurityAuditSeverity::Warn
                },
                title: "Gateway bind is publicly reachable".to_owned(),
                detail: format!("gateway.server.bind is `{bind}` (non-loopback)."),
                remediation: Some(
                    "bind to loopback for local-only control plane or keep strict auth + network controls"
                        .to_owned(),
                ),
            });
        }
        if let Some(http_bind) = cfg.gateway.server.http_bind.as_deref() {
            if !is_loopback_bind(http_bind.trim()) {
                findings.push(SecurityAuditFinding {
                    check_id: "gateway.http_bind.public".to_owned(),
                    severity: if auth_mode == GatewayAuthMode::None {
                        SecurityAuditSeverity::Critical
                    } else {
                        SecurityAuditSeverity::Warn
                    },
                    title: "Gateway HTTP bind is publicly reachable".to_owned(),
                    detail: format!("gateway.server.http_bind is `{}` (non-loopback).", http_bind.trim()),
                    remediation: Some(
                        "bind HTTP endpoints to loopback unless external access is explicitly required"
                            .to_owned(),
                    ),
                });
            }
        }
    }

    if cfg.runtime.audit_only {
        findings.push(SecurityAuditFinding {
            check_id: "runtime.audit_only.enabled".to_owned(),
            severity: SecurityAuditSeverity::Warn,
            title: "Runtime is in audit-only mode".to_owned(),
            detail: "runtime.audit_only=true converts blocking decisions into review/allow paths."
                .to_owned(),
            remediation: Some(
                "disable audit-only mode for enforcement (`runtime.audit_only=false`)".to_owned(),
            ),
        });
    }

    if cfg.runtime.group_activation_mode == GroupActivationMode::Always {
        findings.push(SecurityAuditFinding {
            check_id: "runtime.group_activation_mode.always".to_owned(),
            severity: SecurityAuditSeverity::Warn,
            title: "Group activation mode is broad".to_owned(),
            detail: "runtime.group_activation_mode is `always`, which increases unsolicited execution in shared group traffic.".to_owned(),
            remediation: Some(
                "set `runtime.group_activation_mode=\"mention\"` for tighter shared-channel activation"
                    .to_owned(),
            ),
        });
    }

    if cfg.security.allowed_command_prefixes.is_empty() {
        findings.push(SecurityAuditFinding {
            check_id: "security.allowed_command_prefixes.empty".to_owned(),
            severity: SecurityAuditSeverity::Critical,
            title: "Allowed command prefix list is empty".to_owned(),
            detail: "security.allowed_command_prefixes has no entries; command allowlist posture is undefined.".to_owned(),
            remediation: Some(
                "restore a minimal allowlist (for example `git `, `ls`, `rg `) before production use"
                    .to_owned(),
            ),
        });
    } else {
        let dangerous = cfg
            .security
            .allowed_command_prefixes
            .iter()
            .filter(|prefix| is_dangerous_prefix(prefix))
            .cloned()
            .collect::<Vec<_>>();
        if !dangerous.is_empty() {
            findings.push(SecurityAuditFinding {
                check_id: "security.allowed_command_prefixes.dangerous".to_owned(),
                severity: SecurityAuditSeverity::Critical,
                title: "Allowlist includes dangerous command prefixes".to_owned(),
                detail: format!("dangerous prefixes detected: {}", dangerous.join(", ")),
                remediation: Some(
                    "remove destructive or shell-bootstrap prefixes from allowlist".to_owned(),
                ),
            });
        }
    }

    if cfg.security.blocked_command_patterns.is_empty() {
        findings.push(SecurityAuditFinding {
            check_id: "security.blocked_command_patterns.empty".to_owned(),
            severity: SecurityAuditSeverity::Warn,
            title: "Blocked command patterns are empty".to_owned(),
            detail: "security.blocked_command_patterns has no deny signatures.".to_owned(),
            remediation: Some(
                "restore baseline high-risk deny patterns (rm -rf, mkfs, dd, curl|sh)".to_owned(),
            ),
        });
    }

    if !cfg.security.tool_runtime_policy.wasm.enabled {
        findings.push(SecurityAuditFinding {
            check_id: "security.tool_runtime_policy.wasm.disabled".to_owned(),
            severity: SecurityAuditSeverity::Warn,
            title: "Wasm runtime policy is disabled".to_owned(),
            detail: "security.tool_runtime_policy.wasm.enabled=false; live wasm tool policy controls are inactive.".to_owned(),
            remediation: Some("enable wasm policy runtime if wasm tools are expected in production".to_owned()),
        });
    }

    if cfg.security.policy_bundle_path.is_none() {
        findings.push(SecurityAuditFinding {
            check_id: "security.policy_bundle.unset".to_owned(),
            severity: SecurityAuditSeverity::Info,
            title: "Signed policy bundle is not configured".to_owned(),
            detail: "security.policy_bundle_path is unset.".to_owned(),
            remediation: Some(
                "configure a signed policy bundle for tamper-evident policy rollouts".to_owned(),
            ),
        });
    }

    collect_filesystem_findings(cfg, config_path, findings);
}

fn collect_filesystem_findings(
    cfg: &Config,
    config_path: &Path,
    findings: &mut Vec<SecurityAuditFinding>,
) {
    let resolved_config = resolve_path(config_path, config_path);
    collect_path_permission_findings(findings, &resolved_config, "fs.config", "config file", true);

    let session_state_path = resolve_path(config_path, &cfg.runtime.session_state_path);
    collect_path_permission_findings(
        findings,
        &session_state_path,
        "fs.state_file",
        "session state file",
        true,
    );

    if let Some(parent) = session_state_path.parent() {
        collect_path_permission_findings(
            findings,
            parent,
            "fs.state_dir",
            "session state directory",
            false,
        );
    }

    let quarantine_dir = resolve_path(config_path, &cfg.security.quarantine_dir);
    collect_path_permission_findings(
        findings,
        &quarantine_dir,
        "fs.quarantine_dir",
        "quarantine directory",
        false,
    );
}

#[allow(clippy::needless_return)]
fn collect_path_permission_findings(
    findings: &mut Vec<SecurityAuditFinding>,
    target: &Path,
    check_prefix: &str,
    label: &str,
    expect_file: bool,
) {
    let display = target.display().to_string();
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::NotFound {
                findings.push(SecurityAuditFinding {
                    check_id: format!("{check_prefix}.missing"),
                    severity: SecurityAuditSeverity::Info,
                    title: format!("{} does not exist yet", capitalize(label)),
                    detail: format!("{label} path `{display}` is missing."),
                    remediation: Some("initialize runtime state and rerun the audit".to_owned()),
                });
            } else {
                findings.push(SecurityAuditFinding {
                    check_id: format!("{check_prefix}.stat_failed"),
                    severity: SecurityAuditSeverity::Warn,
                    title: format!("Failed to inspect {}", label),
                    detail: format!("could not stat `{display}`: {err}"),
                    remediation: None,
                });
            }
            return;
        }
    };

    if metadata.file_type().is_symlink() {
        findings.push(SecurityAuditFinding {
            check_id: format!("{check_prefix}.symlink"),
            severity: SecurityAuditSeverity::Warn,
            title: format!("{} is a symlink", capitalize(label)),
            detail: format!(
                "`{display}` is a symlink; treat target path as a separate trust boundary."
            ),
            remediation: None,
        });
    }

    if expect_file && metadata.is_dir() {
        findings.push(SecurityAuditFinding {
            check_id: format!("{check_prefix}.is_dir"),
            severity: SecurityAuditSeverity::Warn,
            title: format!("{} is a directory", capitalize(label)),
            detail: format!("expected file path, but `{display}` is a directory."),
            remediation: Some("set this path to a file location".to_owned()),
        });
        return;
    }

    if !expect_file && metadata.is_file() {
        findings.push(SecurityAuditFinding {
            check_id: format!("{check_prefix}.is_file"),
            severity: SecurityAuditSeverity::Warn,
            title: format!("{} is a file", capitalize(label)),
            detail: format!("expected directory path, but `{display}` is a file."),
            remediation: Some("set this path to a directory location".to_owned()),
        });
        return;
    }

    #[cfg(unix)]
    {
        let mode = metadata.permissions().mode() & 0o777;
        let world_writable = mode & 0o002 != 0;
        let group_writable = mode & 0o020 != 0;
        let world_readable = mode & 0o004 != 0;
        let group_readable = mode & 0o040 != 0;
        let is_secret = expect_file;

        if world_writable {
            findings.push(SecurityAuditFinding {
                check_id: format!("{check_prefix}.world_writable"),
                severity: SecurityAuditSeverity::Critical,
                title: format!("{} is world-writable", capitalize(label)),
                detail: format!("`{display}` mode is {:o}; other users can modify it.", mode),
                remediation: Some(expected_mode_hint(expect_file).to_owned()),
            });
        } else if group_writable {
            findings.push(SecurityAuditFinding {
                check_id: format!("{check_prefix}.group_writable"),
                severity: SecurityAuditSeverity::Warn,
                title: format!("{} is group-writable", capitalize(label)),
                detail: format!(
                    "`{display}` mode is {:o}; group members can modify it.",
                    mode
                ),
                remediation: Some(expected_mode_hint(expect_file).to_owned()),
            });
        }

        if is_secret && world_readable {
            findings.push(SecurityAuditFinding {
                check_id: format!("{check_prefix}.world_readable"),
                severity: SecurityAuditSeverity::Critical,
                title: format!("{} is world-readable", capitalize(label)),
                detail: format!(
                    "`{display}` mode is {:o}; sensitive data may be exposed.",
                    mode
                ),
                remediation: Some(expected_mode_hint(expect_file).to_owned()),
            });
        } else if is_secret && group_readable {
            findings.push(SecurityAuditFinding {
                check_id: format!("{check_prefix}.group_readable"),
                severity: SecurityAuditSeverity::Warn,
                title: format!("{} is group-readable", capitalize(label)),
                detail: format!(
                    "`{display}` mode is {:o}; sensitive data may be exposed to group users.",
                    mode
                ),
                remediation: Some(expected_mode_hint(expect_file).to_owned()),
            });
        }
    }
}

fn apply_security_fixes(config_path: &Path, cfg: &Config) -> SecurityFixResult {
    let mut next_cfg = cfg.clone();
    let mut changes = Vec::new();
    let mut actions = Vec::new();
    let mut errors = Vec::new();

    if next_cfg.gateway.server.auth_mode == GatewayAuthMode::None {
        next_cfg.gateway.server.auth_mode = GatewayAuthMode::Auto;
        changes.push("set gateway.server.auth_mode from none to auto".to_owned());
    }
    if next_cfg.runtime.group_activation_mode == GroupActivationMode::Always {
        next_cfg.runtime.group_activation_mode = GroupActivationMode::Mention;
        changes.push("set runtime.group_activation_mode from always to mention".to_owned());
    }
    if next_cfg.security.allowed_command_prefixes.is_empty() {
        next_cfg.security.allowed_command_prefixes =
            Config::default().security.allowed_command_prefixes;
        changes.push("restored default security.allowed_command_prefixes".to_owned());
    }
    if next_cfg.security.blocked_command_patterns.is_empty() {
        next_cfg.security.blocked_command_patterns =
            Config::default().security.blocked_command_patterns;
        changes.push("restored default security.blocked_command_patterns".to_owned());
    }

    if !changes.is_empty() {
        let path = resolve_path(config_path, config_path);
        if let Some(parent) = path.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                errors.push(format!(
                    "failed to create config parent directory {}: {err}",
                    parent.display()
                ));
            }
        }
        match toml::to_string_pretty(&next_cfg) {
            Ok(text) => match fs::write(&path, text) {
                Ok(_) => actions.push(SecurityFixAction {
                    kind: "write".to_owned(),
                    target: path.display().to_string(),
                    mode: None,
                    ok: true,
                    skipped: None,
                    error: None,
                }),
                Err(err) => {
                    errors.push(format!("failed to write config {}: {err}", path.display()));
                    actions.push(SecurityFixAction {
                        kind: "write".to_owned(),
                        target: path.display().to_string(),
                        mode: None,
                        ok: false,
                        skipped: None,
                        error: Some(err.to_string()),
                    });
                }
            },
            Err(err) => errors.push(format!("failed to serialize config: {err}")),
        }
    }

    let config_file = resolve_path(config_path, config_path);
    let session_state_path = resolve_path(config_path, &next_cfg.runtime.session_state_path);
    let session_state_dir = session_state_path.parent().map(Path::to_path_buf);
    let quarantine_dir = resolve_path(config_path, &next_cfg.security.quarantine_dir);

    actions.push(tighten_permissions(&config_file, false, 0o600));
    if let Some(session_state_dir) = session_state_dir {
        actions.push(tighten_permissions(&session_state_dir, true, 0o700));
    }
    actions.push(tighten_permissions(&session_state_path, false, 0o600));
    actions.push(tighten_permissions(&quarantine_dir, true, 0o700));

    let ok = errors.is_empty()
        && actions
            .iter()
            .all(|action| action.ok || action.skipped.is_some());
    SecurityFixResult {
        ok,
        changes,
        actions,
        errors,
    }
}

fn tighten_permissions(path: &Path, is_dir: bool, mode: u32) -> SecurityFixAction {
    let target = path.display().to_string();
    if !path.exists() {
        return SecurityFixAction {
            kind: "chmod".to_owned(),
            target,
            mode: Some(format!("{mode:o}")),
            ok: false,
            skipped: Some("path does not exist".to_owned()),
            error: None,
        };
    }

    #[cfg(unix)]
    {
        if is_dir && !path.is_dir() {
            return SecurityFixAction {
                kind: "chmod".to_owned(),
                target,
                mode: Some(format!("{mode:o}")),
                ok: false,
                skipped: Some("target is not a directory".to_owned()),
                error: None,
            };
        }
        if !is_dir && path.is_dir() {
            return SecurityFixAction {
                kind: "chmod".to_owned(),
                target,
                mode: Some(format!("{mode:o}")),
                ok: false,
                skipped: Some("target is a directory".to_owned()),
                error: None,
            };
        }

        match fs::metadata(path) {
            Ok(metadata) => {
                let mut perms = metadata.permissions();
                perms.set_mode(mode);
                match fs::set_permissions(path, perms) {
                    Ok(_) => SecurityFixAction {
                        kind: "chmod".to_owned(),
                        target,
                        mode: Some(format!("{mode:o}")),
                        ok: true,
                        skipped: None,
                        error: None,
                    },
                    Err(err) => SecurityFixAction {
                        kind: "chmod".to_owned(),
                        target,
                        mode: Some(format!("{mode:o}")),
                        ok: false,
                        skipped: None,
                        error: Some(err.to_string()),
                    },
                }
            }
            Err(err) => SecurityFixAction {
                kind: "chmod".to_owned(),
                target,
                mode: Some(format!("{mode:o}")),
                ok: false,
                skipped: None,
                error: Some(err.to_string()),
            },
        }
    }

    #[cfg(not(unix))]
    {
        let _ = (is_dir, mode);
        SecurityFixAction {
            kind: "chmod".to_owned(),
            target,
            mode: Some(format!("{mode:o}")),
            ok: false,
            skipped: Some("chmod fix is only applied on unix hosts".to_owned()),
            error: None,
        }
    }
}

async fn deep_probe_gateway(cfg: &Config) -> SecurityAuditDeepGateway {
    let gateway_url = cfg.gateway.url.trim();
    if gateway_url.is_empty() {
        return SecurityAuditDeepGateway {
            attempted: false,
            url: None,
            ok: false,
            error: Some("gateway.url is empty".to_owned()),
        };
    }

    let (host, port) = match parse_gateway_target(gateway_url) {
        Ok(value) => value,
        Err(err) => {
            return SecurityAuditDeepGateway {
                attempted: false,
                url: Some(gateway_url.to_owned()),
                ok: false,
                error: Some(err),
            };
        }
    };

    let timeout_ms = 1_500;
    match timeout(
        Duration::from_millis(timeout_ms),
        TcpStream::connect((host.as_str(), port)),
    )
    .await
    {
        Ok(Ok(_stream)) => SecurityAuditDeepGateway {
            attempted: true,
            url: Some(gateway_url.to_owned()),
            ok: true,
            error: None,
        },
        Ok(Err(err)) => SecurityAuditDeepGateway {
            attempted: true,
            url: Some(gateway_url.to_owned()),
            ok: false,
            error: Some(format!("failed to connect to {host}:{port}: {err}")),
        },
        Err(_) => SecurityAuditDeepGateway {
            attempted: true,
            url: Some(gateway_url.to_owned()),
            ok: false,
            error: Some(format!("gateway probe timed out after {timeout_ms} ms")),
        },
    }
}

fn parse_gateway_target(url_text: &str) -> Result<(String, u16), String> {
    let url = Url::parse(url_text)
        .map_err(|err| format!("failed to parse gateway.url `{url_text}`: {err}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| format!("gateway.url `{url_text}` does not include a host"))?
        .to_owned();

    let default_port = match url.scheme() {
        "ws" | "http" => Some(80),
        "wss" | "https" => Some(443),
        _ => None,
    };
    let port = url
        .port()
        .or(default_port)
        .ok_or_else(|| format!("gateway.url `{url_text}` does not include a port"))?;

    Ok((host, port))
}

fn resolve_path(config_path: &Path, candidate: &Path) -> PathBuf {
    if candidate.is_absolute() {
        return candidate.to_path_buf();
    }

    if candidate == config_path {
        return std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(candidate);
    }

    let base = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    base.join(candidate)
}

fn is_loopback_bind(bind: &str) -> bool {
    let host = bind_host(bind);
    is_loopback_host(&host)
}

fn bind_host(bind: &str) -> String {
    let trimmed = bind.trim();
    if let Some(rest) = trimmed.strip_prefix('[') {
        if let Some((host, _)) = rest.split_once(']') {
            return host.to_owned();
        }
    }
    if trimmed.contains("://") {
        if let Ok(url) = Url::parse(trimmed) {
            if let Some(host) = url.host_str() {
                return host.to_owned();
            }
        }
    }
    if let Some((host, _port)) = trimmed.rsplit_once(':') {
        if !host.is_empty() && !host.contains(':') {
            return host.to_owned();
        }
    }
    trimmed.to_owned()
}

fn is_loopback_host(host: &str) -> bool {
    matches!(
        host.trim().to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
}

fn is_dangerous_prefix(prefix: &str) -> bool {
    let normalized = prefix.trim().to_ascii_lowercase();
    let dangerous = [
        "rm ",
        "del ",
        "sudo ",
        "mkfs",
        "dd ",
        "chmod 777",
        "curl ",
        "wget ",
        "powershell ",
        "invoke-expression",
        "iex ",
    ];
    dangerous.iter().any(|item| normalized.starts_with(item))
}

#[cfg(unix)]
fn expected_mode_hint(expect_file: bool) -> &'static str {
    if expect_file {
        "restrict permissions to 600"
    } else {
        "restrict permissions to 700"
    }
}

fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut output = String::new();
    output.extend(first.to_uppercase());
    output.push_str(chars.as_str());
    output
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(1);

    fn temp_dir(label: &str) -> PathBuf {
        let id = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "openclaw-rs-security-audit-{label}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create test temp dir");
        path
    }

    #[tokio::test]
    async fn security_audit_reports_critical_when_auth_is_none() {
        let root = temp_dir("auth-none");
        let config_path = root.join("openclaw-rs.toml");
        let mut cfg = Config::default();
        cfg.gateway.server.auth_mode = GatewayAuthMode::None;
        fs::write(
            &config_path,
            toml::to_string_pretty(&cfg).expect("serialize config"),
        )
        .expect("write config");

        let run = run_security_audit(
            &config_path,
            SecurityAuditOptions {
                deep: false,
                fix: false,
            },
        )
        .await;

        assert!(run
            .report
            .findings
            .iter()
            .any(|finding| finding.check_id == "gateway.auth.none"));
        assert!(run.report.summary.critical >= 1);
    }

    #[tokio::test]
    async fn security_audit_fix_mode_updates_unsafe_defaults() {
        let root = temp_dir("fix");
        let config_path = root.join("openclaw-rs.toml");
        let mut cfg = Config::default();
        cfg.gateway.server.auth_mode = GatewayAuthMode::None;
        cfg.runtime.group_activation_mode = GroupActivationMode::Always;
        cfg.security.allowed_command_prefixes.clear();
        cfg.security.blocked_command_patterns.clear();
        fs::write(
            &config_path,
            toml::to_string_pretty(&cfg).expect("serialize config"),
        )
        .expect("write config");

        let run = run_security_audit(
            &config_path,
            SecurityAuditOptions {
                deep: false,
                fix: true,
            },
        )
        .await;

        let fix = run.fix.as_ref().expect("fix result");
        assert!(fix.ok);
        assert!(!fix.changes.is_empty());

        let fixed = Config::load(&config_path).expect("reload fixed config");
        assert_eq!(fixed.gateway.server.auth_mode, GatewayAuthMode::Auto);
        assert_eq!(
            fixed.runtime.group_activation_mode,
            GroupActivationMode::Mention
        );
        assert!(!fixed.security.allowed_command_prefixes.is_empty());
        assert!(!fixed.security.blocked_command_patterns.is_empty());
    }

    #[test]
    fn bind_loopback_detection_handles_ipv4_and_ipv6() {
        assert!(is_loopback_bind("127.0.0.1:18789"));
        assert!(is_loopback_bind("[::1]:18789"));
        assert!(!is_loopback_bind("0.0.0.0:18789"));
    }

    #[test]
    fn parse_gateway_target_accepts_ws_and_wss_defaults() {
        let ws = parse_gateway_target("ws://localhost/ws").expect("ws target");
        assert_eq!(ws.0, "localhost");
        assert_eq!(ws.1, 80);

        let wss = parse_gateway_target("wss://example.com/socket").expect("wss target");
        assert_eq!(wss.0, "example.com");
        assert_eq!(wss.1, 443);
    }
}
