use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub gateway: GatewayConfig,
    pub runtime: RuntimeConfig,
    pub security: SecurityConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    pub url: String,
    pub token: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default = "default_gateway_runtime_mode")]
    pub runtime_mode: GatewayRuntimeMode,
    #[serde(default)]
    pub server: GatewayServerConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayServerConfig {
    #[serde(default = "default_gateway_server_bind")]
    pub bind: String,
    #[serde(default)]
    pub http_bind: Option<String>,
    #[serde(default = "default_gateway_auth_mode")]
    pub auth_mode: GatewayAuthMode,
    #[serde(default = "default_gateway_handshake_timeout_ms")]
    pub handshake_timeout_ms: u64,
    #[serde(default = "default_gateway_event_queue_capacity")]
    pub event_queue_capacity: usize,
    #[serde(default = "default_gateway_reload_interval_secs")]
    pub reload_interval_secs: u64,
    #[serde(default = "default_gateway_tick_interval_ms")]
    pub tick_interval_ms: u64,
}

impl Default for GatewayServerConfig {
    fn default() -> Self {
        Self {
            bind: default_gateway_server_bind(),
            http_bind: None,
            auth_mode: default_gateway_auth_mode(),
            handshake_timeout_ms: default_gateway_handshake_timeout_ms(),
            event_queue_capacity: default_gateway_event_queue_capacity(),
            reload_interval_secs: default_gateway_reload_interval_secs(),
            tick_interval_ms: default_gateway_tick_interval_ms(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayRuntimeMode {
    BridgeClient,
    StandaloneServer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayAuthMode {
    Auto,
    None,
    Token,
    Password,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub audit_only: bool,
    pub decision_event: String,
    pub worker_concurrency: usize,
    pub max_queue: usize,
    #[serde(default = "default_session_queue_mode")]
    pub session_queue_mode: SessionQueueMode,
    #[serde(default = "default_group_activation_mode")]
    pub group_activation_mode: GroupActivationMode,
    pub eval_timeout_ms: u64,
    pub memory_sample_secs: u64,
    #[serde(default = "default_idempotency_ttl_secs")]
    pub idempotency_ttl_secs: u64,
    #[serde(default = "default_idempotency_max_entries")]
    pub idempotency_max_entries: usize,
    #[serde(default = "default_session_state_path")]
    pub session_state_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    pub review_threshold: u8,
    pub block_threshold: u8,
    pub virustotal_api_key: Option<String>,
    pub virustotal_timeout_ms: u64,
    #[serde(default)]
    pub edr_telemetry_path: Option<PathBuf>,
    #[serde(default = "default_edr_telemetry_max_age_secs")]
    pub edr_telemetry_max_age_secs: u64,
    #[serde(default = "default_edr_high_risk_tags")]
    pub edr_high_risk_tags: Vec<String>,
    #[serde(default = "default_edr_telemetry_risk_bonus")]
    pub edr_telemetry_risk_bonus: u8,
    #[serde(default)]
    pub policy_bundle_path: Option<PathBuf>,
    #[serde(default)]
    pub policy_bundle_key: Option<String>,
    #[serde(default)]
    pub policy_bundle_keys: HashMap<String, String>,
    #[serde(default)]
    pub attestation_expected_sha256: Option<String>,
    #[serde(default = "default_attestation_mismatch_risk_bonus")]
    pub attestation_mismatch_risk_bonus: u8,
    #[serde(default)]
    pub attestation_report_path: Option<PathBuf>,
    #[serde(default)]
    pub attestation_hmac_key: Option<String>,
    pub quarantine_dir: PathBuf,
    pub protect_paths: Vec<PathBuf>,
    pub allowed_command_prefixes: Vec<String>,
    pub blocked_command_patterns: Vec<String>,
    pub prompt_injection_patterns: Vec<String>,
    #[serde(default)]
    pub tool_policies: HashMap<String, PolicyAction>,
    #[serde(default = "default_tool_risk_bonus")]
    pub tool_risk_bonus: HashMap<String, u8>,
    #[serde(default = "default_channel_risk_bonus")]
    pub channel_risk_bonus: HashMap<String, u8>,
    #[serde(default)]
    pub tool_runtime_policy: ToolRuntimePolicyConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyAction {
    Allow,
    Review,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionQueueMode {
    Followup,
    Steer,
    Collect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupActivationMode {
    Mention,
    Always,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolRuntimePolicyConfig {
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default, rename = "byProvider", alias = "by_provider")]
    pub by_provider: HashMap<String, ToolRuntimePolicyRule>,
    #[serde(default)]
    pub loop_detection: ToolLoopDetectionConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolRuntimePolicyRule {
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolLoopDetectionConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_tool_loop_history_size")]
    pub history_size: usize,
    #[serde(default = "default_tool_loop_warning_threshold")]
    pub warning_threshold: usize,
    #[serde(default = "default_tool_loop_critical_threshold")]
    pub critical_threshold: usize,
}

impl Default for ToolLoopDetectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            history_size: default_tool_loop_history_size(),
            warning_threshold: default_tool_loop_warning_threshold(),
            critical_threshold: default_tool_loop_critical_threshold(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            gateway: GatewayConfig {
                url: "ws://127.0.0.1:18789/ws".to_owned(),
                token: None,
                password: None,
                runtime_mode: default_gateway_runtime_mode(),
                server: GatewayServerConfig {
                    bind: default_gateway_server_bind(),
                    http_bind: None,
                    auth_mode: default_gateway_auth_mode(),
                    handshake_timeout_ms: default_gateway_handshake_timeout_ms(),
                    event_queue_capacity: default_gateway_event_queue_capacity(),
                    reload_interval_secs: default_gateway_reload_interval_secs(),
                    tick_interval_ms: default_gateway_tick_interval_ms(),
                },
            },
            runtime: RuntimeConfig {
                audit_only: false,
                decision_event: "security.decision".to_owned(),
                worker_concurrency: 8,
                max_queue: 256,
                session_queue_mode: default_session_queue_mode(),
                group_activation_mode: default_group_activation_mode(),
                eval_timeout_ms: 2_500,
                memory_sample_secs: 15,
                idempotency_ttl_secs: default_idempotency_ttl_secs(),
                idempotency_max_entries: default_idempotency_max_entries(),
                session_state_path: default_session_state_path(),
            },
            security: SecurityConfig {
                review_threshold: 35,
                block_threshold: 65,
                virustotal_api_key: None,
                virustotal_timeout_ms: 1_400,
                edr_telemetry_path: None,
                edr_telemetry_max_age_secs: default_edr_telemetry_max_age_secs(),
                edr_high_risk_tags: default_edr_high_risk_tags(),
                edr_telemetry_risk_bonus: default_edr_telemetry_risk_bonus(),
                policy_bundle_path: None,
                policy_bundle_key: None,
                policy_bundle_keys: HashMap::new(),
                attestation_expected_sha256: None,
                attestation_mismatch_risk_bonus: default_attestation_mismatch_risk_bonus(),
                attestation_report_path: None,
                attestation_hmac_key: None,
                quarantine_dir: PathBuf::from(".openclaw-rs/quarantine"),
                protect_paths: vec![
                    PathBuf::from("./openclaw.mjs"),
                    PathBuf::from("./dist/index.js"),
                ],
                allowed_command_prefixes: vec![
                    "git ".to_owned(),
                    "ls".to_owned(),
                    "rg ".to_owned(),
                ],
                blocked_command_patterns: vec![
                    r"(?i)\brm\s+-rf\s+/".to_owned(),
                    r"(?i)\bmkfs\b".to_owned(),
                    r"(?i)\bdd\s+if=".to_owned(),
                    r"(?i)\bcurl\s+[^|]*\|\s*sh\b".to_owned(),
                    r"(?i)\bwget\s+[^|]*\|\s*sh\b".to_owned(),
                ],
                prompt_injection_patterns: vec![
                    r"(?i)ignore\s+all\s+previous\s+instructions".to_owned(),
                    r"(?i)reveal\s+the\s+system\s+prompt".to_owned(),
                    r"(?i)override\s+developer\s+instructions".to_owned(),
                    r"(?i)disable\s+safety".to_owned(),
                ],
                tool_policies: HashMap::new(),
                tool_risk_bonus: default_tool_risk_bonus(),
                channel_risk_bonus: default_channel_risk_bonus(),
                tool_runtime_policy: ToolRuntimePolicyConfig::default(),
            },
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let mut cfg = if path.exists() {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("failed reading config file {}", path.display()))?;
            toml::from_str::<Config>(&text)
                .with_context(|| format!("failed parsing TOML config {}", path.display()))?
        } else {
            Self::default()
        };
        cfg.apply_env_overrides();
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn apply_cli_overrides(
        &mut self,
        gateway_url: Option<&str>,
        gateway_token: Option<&str>,
        audit_only: bool,
    ) {
        if let Some(url) = gateway_url {
            self.gateway.url = url.to_owned();
        }
        if let Some(token) = gateway_token {
            self.gateway.token = Some(token.to_owned());
        }
        if audit_only {
            self.runtime.audit_only = true;
        }
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(v) = env::var("OPENCLAW_RS_GATEWAY_URL") {
            self.gateway.url = v;
        }
        if let Ok(v) = env::var("OPENCLAW_RS_GATEWAY_TOKEN") {
            self.gateway.token = Some(v);
        }
        if let Ok(v) = env::var("OPENCLAW_RS_GATEWAY_PASSWORD") {
            self.gateway.password = Some(v);
        }
        if let Ok(v) = env::var("OPENCLAW_RS_GATEWAY_RUNTIME_MODE") {
            if let Some(mode) = parse_gateway_runtime_mode(&v) {
                self.gateway.runtime_mode = mode;
            }
        }
        if let Ok(v) = env::var("OPENCLAW_RS_GATEWAY_AUTH_MODE") {
            if let Some(mode) = parse_gateway_auth_mode(&v) {
                self.gateway.server.auth_mode = mode;
            }
        }
        if let Ok(v) = env::var("OPENCLAW_RS_GATEWAY_SERVER_BIND") {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                self.gateway.server.bind = trimmed.to_owned();
            }
        }
        if let Ok(v) = env::var("OPENCLAW_RS_GATEWAY_HTTP_BIND") {
            let trimmed = v.trim();
            self.gateway.server.http_bind = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            };
        }
        if let Ok(v) = env::var("OPENCLAW_RS_GATEWAY_EVENT_QUEUE_CAPACITY") {
            if let Ok(n) = v.parse::<usize>() {
                self.gateway.server.event_queue_capacity = n.max(8);
            }
        }
        if let Ok(v) = env::var("OPENCLAW_RS_GATEWAY_HANDSHAKE_TIMEOUT_MS") {
            if let Ok(n) = v.parse::<u64>() {
                self.gateway.server.handshake_timeout_ms = n.max(500);
            }
        }
        if let Ok(v) = env::var("OPENCLAW_RS_GATEWAY_RELOAD_INTERVAL_SECS") {
            if let Ok(n) = v.parse::<u64>() {
                self.gateway.server.reload_interval_secs = n;
            }
        }
        if let Ok(v) = env::var("OPENCLAW_RS_GATEWAY_TICK_INTERVAL_MS") {
            if let Ok(n) = v.parse::<u64>() {
                self.gateway.server.tick_interval_ms = n.max(250);
            }
        }
        if let Ok(v) = env::var("OPENCLAW_RS_AUDIT_ONLY") {
            self.runtime.audit_only = parse_bool(&v);
        }
        if let Ok(v) = env::var("OPENCLAW_RS_VT_API_KEY") {
            self.security.virustotal_api_key = Some(v);
        }
        if let Ok(v) = env::var("OPENCLAW_RS_EDR_TELEMETRY_PATH") {
            let trimmed = v.trim();
            self.security.edr_telemetry_path = if trimmed.is_empty() {
                None
            } else {
                Some(PathBuf::from(trimmed))
            };
        }
        if let Ok(v) = env::var("OPENCLAW_RS_EDR_TELEMETRY_MAX_AGE_SECS") {
            if let Ok(n) = v.parse::<u64>() {
                self.security.edr_telemetry_max_age_secs = n.max(1);
            }
        }
        if let Ok(v) = env::var("OPENCLAW_RS_EDR_HIGH_RISK_TAGS") {
            self.security.edr_high_risk_tags = split_csv(&v);
        }
        if let Ok(v) = env::var("OPENCLAW_RS_EDR_TELEMETRY_RISK_BONUS") {
            if let Ok(n) = v.parse::<u8>() {
                self.security.edr_telemetry_risk_bonus = n.max(1);
            }
        }
        if let Ok(v) = env::var("OPENCLAW_RS_ATTESTATION_EXPECTED_SHA256") {
            let trimmed = v.trim();
            self.security.attestation_expected_sha256 = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            };
        }
        if let Ok(v) = env::var("OPENCLAW_RS_ATTESTATION_MISMATCH_RISK_BONUS") {
            if let Ok(n) = v.parse::<u8>() {
                self.security.attestation_mismatch_risk_bonus = n.max(1);
            }
        }
        if let Ok(v) = env::var("OPENCLAW_RS_ATTESTATION_REPORT_PATH") {
            let trimmed = v.trim();
            self.security.attestation_report_path = if trimmed.is_empty() {
                None
            } else {
                Some(PathBuf::from(trimmed))
            };
        }
        if let Ok(v) = env::var("OPENCLAW_RS_ATTESTATION_HMAC_KEY") {
            let trimmed = v.trim();
            self.security.attestation_hmac_key = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            };
        }
        if let Ok(v) = env::var("OPENCLAW_RS_POLICY_BUNDLE_PATH") {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                self.security.policy_bundle_path = None;
            } else {
                self.security.policy_bundle_path = Some(PathBuf::from(trimmed));
            }
        }
        if let Ok(v) = env::var("OPENCLAW_RS_POLICY_BUNDLE_KEY") {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                self.security.policy_bundle_key = None;
            } else {
                self.security.policy_bundle_key = Some(trimmed.to_owned());
            }
        }
        if let Ok(v) = env::var("OPENCLAW_RS_POLICY_BUNDLE_KEYS") {
            self.security.policy_bundle_keys = parse_keyed_csv_map(&v);
        }
        if let Ok(v) = env::var("OPENCLAW_RS_WORKER_CONCURRENCY") {
            if let Ok(n) = v.parse::<usize>() {
                self.runtime.worker_concurrency = n.max(1);
            }
        }
        if let Ok(v) = env::var("OPENCLAW_RS_MAX_QUEUE") {
            if let Ok(n) = v.parse::<usize>() {
                self.runtime.max_queue = n.max(16);
            }
        }
        if let Ok(v) = env::var("OPENCLAW_RS_SESSION_QUEUE_MODE") {
            if let Some(mode) = parse_session_queue_mode(&v) {
                self.runtime.session_queue_mode = mode;
            }
        }
        if let Ok(v) = env::var("OPENCLAW_RS_GROUP_ACTIVATION_MODE")
            .or_else(|_| env::var("OPENCLAW_RS_GROUP_ACTIVATION"))
        {
            if let Some(mode) = parse_group_activation_mode(&v) {
                self.runtime.group_activation_mode = mode;
            }
        }
        if let Ok(v) = env::var("OPENCLAW_RS_ALLOWED_COMMAND_PREFIXES") {
            self.security.allowed_command_prefixes = split_csv(&v);
        }
        if let Ok(v) = env::var("OPENCLAW_RS_MEMORY_SAMPLE_SECS") {
            if let Ok(n) = v.parse::<u64>() {
                self.runtime.memory_sample_secs = n.max(1);
            }
        }
        if let Ok(v) = env::var("OPENCLAW_RS_IDEMPOTENCY_TTL_SECS") {
            if let Ok(n) = v.parse::<u64>() {
                self.runtime.idempotency_ttl_secs = n.max(1);
            }
        }
        if let Ok(v) = env::var("OPENCLAW_RS_IDEMPOTENCY_MAX_ENTRIES") {
            if let Ok(n) = v.parse::<usize>() {
                self.runtime.idempotency_max_entries = n.max(32);
            }
        }
        if let Ok(v) = env::var("OPENCLAW_RS_SESSION_STATE_PATH") {
            self.runtime.session_state_path = PathBuf::from(v);
        }
    }

    fn validate(&self) -> Result<()> {
        if self.security.review_threshold >= self.security.block_threshold {
            anyhow::bail!("security.review_threshold must be lower than security.block_threshold");
        }
        if self.runtime.worker_concurrency == 0 {
            anyhow::bail!("runtime.worker_concurrency must be > 0");
        }
        if self.runtime.max_queue == 0 {
            anyhow::bail!("runtime.max_queue must be > 0");
        }
        if self.runtime.memory_sample_secs == 0 {
            anyhow::bail!("runtime.memory_sample_secs must be > 0");
        }
        if self.runtime.idempotency_ttl_secs == 0 {
            anyhow::bail!("runtime.idempotency_ttl_secs must be > 0");
        }
        if self.runtime.idempotency_max_entries == 0 {
            anyhow::bail!("runtime.idempotency_max_entries must be > 0");
        }
        if self.security.edr_telemetry_max_age_secs == 0 {
            anyhow::bail!("security.edr_telemetry_max_age_secs must be > 0");
        }
        if self.security.edr_telemetry_risk_bonus == 0 {
            anyhow::bail!("security.edr_telemetry_risk_bonus must be > 0");
        }
        if self.security.attestation_mismatch_risk_bonus == 0 {
            anyhow::bail!("security.attestation_mismatch_risk_bonus must be > 0");
        }
        if let Some(expected_hash) = self.security.attestation_expected_sha256.as_deref() {
            let normalized = expected_hash.trim().to_ascii_lowercase();
            if !is_sha256_hex(&normalized) {
                anyhow::bail!(
                    "security.attestation_expected_sha256 must be a 64-character hex digest",
                );
            }
        }
        if self.gateway.server.bind.trim().is_empty() {
            anyhow::bail!("gateway.server.bind must not be empty");
        }
        if self
            .gateway
            .server
            .http_bind
            .as_deref()
            .is_some_and(|bind| bind.trim().is_empty())
        {
            anyhow::bail!("gateway.server.http_bind must not be empty when provided");
        }
        if self.gateway.server.event_queue_capacity == 0 {
            anyhow::bail!("gateway.server.event_queue_capacity must be > 0");
        }
        if self.gateway.server.handshake_timeout_ms == 0 {
            anyhow::bail!("gateway.server.handshake_timeout_ms must be > 0");
        }
        if self.gateway.server.tick_interval_ms == 0 {
            anyhow::bail!("gateway.server.tick_interval_ms must be > 0");
        }
        if self
            .security
            .tool_runtime_policy
            .loop_detection
            .warning_threshold
            == 0
        {
            anyhow::bail!(
                "security.tool_runtime_policy.loop_detection.warning_threshold must be > 0"
            );
        }
        if self
            .security
            .tool_runtime_policy
            .loop_detection
            .critical_threshold
            == 0
        {
            anyhow::bail!(
                "security.tool_runtime_policy.loop_detection.critical_threshold must be > 0"
            );
        }
        if self
            .security
            .tool_runtime_policy
            .loop_detection
            .history_size
            == 0
        {
            anyhow::bail!("security.tool_runtime_policy.loop_detection.history_size must be > 0");
        }
        if self
            .security
            .tool_runtime_policy
            .loop_detection
            .critical_threshold
            <= self
                .security
                .tool_runtime_policy
                .loop_detection
                .warning_threshold
        {
            anyhow::bail!(
                "security.tool_runtime_policy.loop_detection.critical_threshold must be greater than warning_threshold"
            );
        }
        match self.gateway.server.auth_mode {
            GatewayAuthMode::Token => {
                let token = self
                    .gateway
                    .token
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or_default();
                if token.is_empty() {
                    anyhow::bail!(
                        "gateway.server.auth_mode=token requires gateway.token or OPENCLAW_RS_GATEWAY_TOKEN",
                    );
                }
            }
            GatewayAuthMode::Password => {
                let password = self
                    .gateway
                    .password
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or_default();
                if password.is_empty() {
                    anyhow::bail!(
                        "gateway.server.auth_mode=password requires gateway.password or OPENCLAW_RS_GATEWAY_PASSWORD",
                    );
                }
            }
            GatewayAuthMode::Auto | GatewayAuthMode::None => {}
        }
        Ok(())
    }
}

fn split_csv(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_keyed_csv_map(input: &str) -> HashMap<String, String> {
    input
        .split(',')
        .filter_map(|entry| {
            let trimmed = entry.trim();
            if trimmed.is_empty() {
                return None;
            }
            let (key, value) = trimmed
                .split_once('=')
                .or_else(|| trimmed.split_once(':'))?;
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim().to_owned();
            if key.is_empty() || value.is_empty() {
                return None;
            }
            Some((key, value))
        })
        .collect()
}

fn parse_bool(s: &str) -> bool {
    matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn default_tool_risk_bonus() -> HashMap<String, u8> {
    HashMap::from([
        ("exec".to_owned(), 20),
        ("bash".to_owned(), 20),
        ("process".to_owned(), 10),
        ("apply_patch".to_owned(), 12),
        ("browser".to_owned(), 8),
        ("gateway".to_owned(), 20),
        ("nodes".to_owned(), 20),
    ])
}

fn default_channel_risk_bonus() -> HashMap<String, u8> {
    HashMap::from([
        ("discord".to_owned(), 10),
        ("slack".to_owned(), 8),
        ("telegram".to_owned(), 6),
        ("whatsapp".to_owned(), 6),
        ("webchat".to_owned(), 8),
    ])
}

fn default_edr_telemetry_max_age_secs() -> u64 {
    300
}

fn default_edr_high_risk_tags() -> Vec<String> {
    vec![
        "ransomware".to_owned(),
        "credential_access".to_owned(),
        "persistence".to_owned(),
        "tamper".to_owned(),
        "remote_access".to_owned(),
        "c2".to_owned(),
    ]
}

fn default_edr_telemetry_risk_bonus() -> u8 {
    45
}

fn default_attestation_mismatch_risk_bonus() -> u8 {
    55
}

fn default_idempotency_ttl_secs() -> u64 {
    300
}

fn default_idempotency_max_entries() -> usize {
    5000
}

fn default_tool_loop_history_size() -> usize {
    30
}

fn default_tool_loop_warning_threshold() -> usize {
    10
}

fn default_tool_loop_critical_threshold() -> usize {
    20
}

fn default_session_state_path() -> PathBuf {
    PathBuf::from(".openclaw-rs/session-state.json")
}

fn default_session_queue_mode() -> SessionQueueMode {
    SessionQueueMode::Followup
}

fn default_group_activation_mode() -> GroupActivationMode {
    GroupActivationMode::Mention
}

fn default_gateway_runtime_mode() -> GatewayRuntimeMode {
    GatewayRuntimeMode::BridgeClient
}

fn default_gateway_server_bind() -> String {
    "127.0.0.1:18789".to_owned()
}

fn default_gateway_auth_mode() -> GatewayAuthMode {
    GatewayAuthMode::Auto
}

fn default_gateway_handshake_timeout_ms() -> u64 {
    10_000
}

fn default_gateway_event_queue_capacity() -> usize {
    256
}

fn default_gateway_reload_interval_secs() -> u64 {
    3
}

fn default_gateway_tick_interval_ms() -> u64 {
    30_000
}

fn parse_session_queue_mode(s: &str) -> Option<SessionQueueMode> {
    match s.trim().to_ascii_lowercase().as_str() {
        "followup" => Some(SessionQueueMode::Followup),
        "steer" => Some(SessionQueueMode::Steer),
        "collect" => Some(SessionQueueMode::Collect),
        _ => None,
    }
}

fn parse_group_activation_mode(s: &str) -> Option<GroupActivationMode> {
    match s.trim().to_ascii_lowercase().as_str() {
        "mention" => Some(GroupActivationMode::Mention),
        "always" => Some(GroupActivationMode::Always),
        _ => None,
    }
}

fn parse_gateway_runtime_mode(s: &str) -> Option<GatewayRuntimeMode> {
    match s.trim().to_ascii_lowercase().as_str() {
        "bridge_client" | "bridge-client" | "client" => Some(GatewayRuntimeMode::BridgeClient),
        "standalone_server" | "standalone-server" | "server" => {
            Some(GatewayRuntimeMode::StandaloneServer)
        }
        _ => None,
    }
}

fn parse_gateway_auth_mode(s: &str) -> Option<GatewayAuthMode> {
    match s.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(GatewayAuthMode::Auto),
        "none" => Some(GatewayAuthMode::None),
        "token" => Some(GatewayAuthMode::Token),
        "password" => Some(GatewayAuthMode::Password),
        _ => None,
    }
}
