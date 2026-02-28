use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::fs;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

use crate::config::GatewayConfig;

const TELEGRAM_POLL_TIMEOUT_SECS: u64 = 20;
const TELEGRAM_HTTP_TIMEOUT_SECS: u64 = 30;
const BRIDGE_RETRY_DELAY_SECS: u64 = 2;
const BRIDGE_IDLE_DELAY_SECS: u64 = 5;
const BRIDGE_STATUS_REFRESH_SECS: u64 = 60;
const AGENT_RPC_TIMEOUT_MS: u64 = 180_000;
const AGENT_WAIT_TIMEOUT_MS: u64 = 70_000;
const TELEGRAM_REPLY_MAX_CHARS: usize = 3_500;
const TELEGRAM_MODEL_HELP_MAX_MODELS: usize = 80;
const TELEGRAM_MODEL_LIST_MAX_MODELS: usize = 120;
const TELEGRAM_AUTH_LIST_MAX_PROVIDERS: usize = 80;
const TELEGRAM_AUTH_BRIDGE_MAX_CANDIDATES: usize = 16;
const TELEGRAM_TTS_TEXT_MAX_CHARS: usize = 2_400;
const TELEGRAM_TTS_INLINE_SAMPLE_MAX_CHARS: usize = 800;
const TELEGRAM_OFFSET_FILE_NAME: &str = "update-offset-default.json";
const TELEGRAM_ACCOUNT_ID: &str = "default";
const TELEGRAM_AUTH_WAIT_DEFAULT_TIMEOUT_MS: u64 = 30_000;
const TELEGRAM_AUTH_WAIT_MIN_TIMEOUT_MS: u64 = 5_000;
const TELEGRAM_AUTH_WAIT_MAX_TIMEOUT_MS: u64 = 300_000;

#[derive(Debug, Clone)]
struct ModelCandidate {
    provider: String,
    model: String,
}

#[derive(Debug, Clone)]
struct CatalogModel {
    provider: String,
    id: String,
    name: String,
}

#[derive(Debug, Clone)]
enum TelegramControlCommand {
    Model { raw_args: String },
    SetApiKey { raw_args: String },
    Auth { raw_args: String },
    Tts { raw_args: String },
}

#[derive(Debug, Clone)]
struct TelegramTtsAudioClip {
    bytes: Vec<u8>,
    output_format: String,
    duration_ms: Option<u64>,
    provider_used: Option<String>,
    source: Option<String>,
    real_audio: bool,
}

#[derive(Debug, Clone)]
struct TelegramSettings {
    bot_token: String,
    dm_policy: String,
    group_policy: String,
    allow_from: Vec<String>,
    candidates: Vec<ModelCandidate>,
}

#[derive(Debug, Clone)]
struct TelegramBotIdentity {
    token: String,
    id: i64,
    username: Option<String>,
}

#[derive(Debug, Clone)]
struct TelegramBridge {
    gateway: GatewayConfig,
    gateway_ws_url: String,
    offset_path: PathBuf,
    legacy_offset_path: PathBuf,
    http: reqwest::Client,
    last_status_emit_ms: u64,
}

pub fn spawn(gateway: GatewayConfig, session_state_path: PathBuf) -> JoinHandle<()> {
    tokio::spawn(async move {
        let bridge = match TelegramBridge::new(gateway, session_state_path) {
            Ok(value) => value,
            Err(err) => {
                warn!("telegram bridge unavailable: {err}");
                return;
            }
        };
        bridge.run_forever().await;
    })
}

impl TelegramBridge {
    fn new(gateway: GatewayConfig, session_state_path: PathBuf) -> Result<Self, String> {
        let gateway_ws_url = derive_gateway_ws_url(&gateway);
        let offset_path = derive_offset_path(&session_state_path);
        let legacy_offset_path = PathBuf::from(".openclaw")
            .join("telegram")
            .join(TELEGRAM_OFFSET_FILE_NAME);
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(TELEGRAM_HTTP_TIMEOUT_SECS))
            .build()
            .map_err(|err| format!("failed building telegram http client: {err}"))?;
        Ok(Self {
            gateway,
            gateway_ws_url,
            offset_path,
            legacy_offset_path,
            http,
            last_status_emit_ms: 0,
        })
    }

    async fn run_forever(mut self) {
        let mut offset = match load_offset(&self.offset_path, &self.legacy_offset_path).await {
            Ok(value) => value,
            Err(err) => {
                warn!("telegram bridge offset read failed: {err}");
                0
            }
        };
        let mut identity: Option<TelegramBotIdentity> = None;

        info!(
            "telegram bridge started (gateway={}, offset_path={})",
            self.gateway_ws_url,
            self.offset_path.display()
        );

        loop {
            let config = match self.fetch_runtime_config().await {
                Ok(value) => value,
                Err(err) => {
                    warn!("telegram bridge config.get failed: {err}");
                    sleep(Duration::from_secs(BRIDGE_RETRY_DELAY_SECS)).await;
                    continue;
                }
            };

            let Some(settings) = extract_telegram_settings(&config) else {
                sleep(Duration::from_secs(BRIDGE_IDLE_DELAY_SECS)).await;
                continue;
            };

            if identity
                .as_ref()
                .map(|cached| cached.token != settings.bot_token)
                .unwrap_or(true)
            {
                match self.fetch_bot_identity(&settings.bot_token).await {
                    Ok(value) => {
                        info!(
                            "telegram bridge linked bot_id={} username={}",
                            value.id,
                            value.username.as_deref().unwrap_or("<unknown>")
                        );
                        identity = Some(value);
                    }
                    Err(err) => {
                        warn!("telegram bridge getMe failed: {err}");
                        sleep(Duration::from_secs(BRIDGE_RETRY_DELAY_SECS)).await;
                        continue;
                    }
                }
            }

            let Some(bot_identity) = identity.clone() else {
                sleep(Duration::from_secs(BRIDGE_RETRY_DELAY_SECS)).await;
                continue;
            };

            let now = now_ms();
            if now.saturating_sub(self.last_status_emit_ms) >= BRIDGE_STATUS_REFRESH_SECS * 1_000 {
                if let Err(err) = self.emit_status_event(&settings).await {
                    debug!("telegram bridge status event failed: {err}");
                } else {
                    self.last_status_emit_ms = now;
                }
            }

            let updates = match self.poll_updates(&settings.bot_token, offset).await {
                Ok(value) => value,
                Err(err) => {
                    warn!("telegram bridge getUpdates failed: {err}");
                    sleep(Duration::from_secs(BRIDGE_RETRY_DELAY_SECS)).await;
                    continue;
                }
            };

            for update in updates {
                let Some(update_id) = update.get("update_id").and_then(Value::as_u64) else {
                    continue;
                };
                if update_id >= offset {
                    offset = update_id.saturating_add(1);
                }
                if let Err(err) = self.save_offset(offset).await {
                    warn!("telegram bridge offset write failed: {err}");
                }
                if let Err(err) = self
                    .process_update(&settings, &bot_identity, &update, update_id)
                    .await
                {
                    warn!("telegram bridge update {} failed: {err}", update_id);
                }
            }
        }
    }

    async fn fetch_runtime_config(&self) -> Result<Value, String> {
        let result = self
            .gateway_rpc_call(
                "config.get",
                json!({}),
                Duration::from_secs(15),
                &["operator.read", "operator.write"],
            )
            .await?;
        let config = result
            .get("config")
            .cloned()
            .ok_or_else(|| "config.get response missing result.config".to_owned())?;
        if !config.is_object() {
            return Err("config.get result.config must be an object".to_owned());
        }
        Ok(config)
    }

    async fn fetch_bot_identity(&self, token: &str) -> Result<TelegramBotIdentity, String> {
        let payload = self.telegram_api(token, "getMe", &[]).await?;
        let id = payload
            .get("id")
            .and_then(Value::as_i64)
            .ok_or_else(|| "telegram getMe missing result.id".to_owned())?;
        let username = payload
            .get("username")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text);
        Ok(TelegramBotIdentity {
            token: token.to_owned(),
            id,
            username,
        })
    }

    async fn poll_updates(&self, token: &str, offset: u64) -> Result<Vec<Value>, String> {
        let mut query = vec![
            ("timeout", TELEGRAM_POLL_TIMEOUT_SECS.to_string()),
            ("allowed_updates", "[\"message\"]".to_owned()),
        ];
        if offset > 0 {
            query.push(("offset", offset.to_string()));
        }
        let result = self.telegram_api(token, "getUpdates", &query).await?;
        let updates = result
            .as_array()
            .cloned()
            .ok_or_else(|| "telegram getUpdates result must be an array".to_owned())?;
        Ok(updates)
    }

    async fn telegram_api(
        &self,
        token: &str,
        method: &str,
        query: &[(&str, String)],
    ) -> Result<Value, String> {
        let base = format!("https://api.telegram.org/bot{token}/{method}");
        let response = self
            .http
            .get(base)
            .query(query)
            .send()
            .await
            .map_err(|err| format!("telegram {method} request failed: {err}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|err| format!("telegram {method} body read failed: {err}"))?;
        if !status.is_success() {
            return Err(format!(
                "telegram {method} returned status {}: {}",
                status.as_u16(),
                truncate_text(&body, 256)
            ));
        }
        let payload: Value = serde_json::from_str(&body)
            .map_err(|err| format!("telegram {method} invalid JSON: {err}"))?;
        if !payload.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            let reason = payload
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("telegram API returned ok=false");
            return Err(format!("telegram {method} failed: {reason}"));
        }
        Ok(payload.get("result").cloned().unwrap_or(Value::Null))
    }

    #[allow(clippy::too_many_arguments)]
    async fn telegram_api_multipart(
        &self,
        token: &str,
        method: &str,
        fields: &[(&str, String)],
        file_field: &str,
        file_name: &str,
        mime: &str,
        file_bytes: Vec<u8>,
    ) -> Result<Value, String> {
        let base = format!("https://api.telegram.org/bot{token}/{method}");
        let mut form = reqwest::multipart::Form::new();
        for (key, value) in fields {
            form = form.text((*key).to_owned(), value.clone());
        }
        let part = reqwest::multipart::Part::bytes(file_bytes)
            .file_name(file_name.to_owned())
            .mime_str(mime)
            .map_err(|err| format!("telegram {method} invalid mime `{mime}`: {err}"))?;
        form = form.part(file_field.to_owned(), part);
        let response = self
            .http
            .post(base)
            .multipart(form)
            .send()
            .await
            .map_err(|err| format!("telegram {method} request failed: {err}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|err| format!("telegram {method} body read failed: {err}"))?;
        if !status.is_success() {
            return Err(format!(
                "telegram {method} returned status {}: {}",
                status.as_u16(),
                truncate_text(&body, 256)
            ));
        }
        let payload: Value = serde_json::from_str(&body)
            .map_err(|err| format!("telegram {method} invalid JSON: {err}"))?;
        if !payload.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            let reason = payload
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("telegram API returned ok=false");
            return Err(format!("telegram {method} failed: {reason}"));
        }
        Ok(payload.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn send_message(
        &self,
        token: &str,
        chat_id: i64,
        reply_text: &str,
        reply_to_message_id: Option<i64>,
    ) -> Result<(), String> {
        let mut query = vec![
            ("chat_id", chat_id.to_string()),
            ("text", truncate_text(reply_text, TELEGRAM_REPLY_MAX_CHARS)),
            ("disable_web_page_preview", "true".to_owned()),
        ];
        if let Some(value) = reply_to_message_id {
            query.push(("reply_to_message_id", value.to_string()));
        }
        let _ = self.telegram_api(token, "sendMessage", &query).await?;
        Ok(())
    }

    async fn send_audio_clip(
        &self,
        token: &str,
        chat_id: i64,
        caption: Option<&str>,
        reply_to_message_id: Option<i64>,
        clip: TelegramTtsAudioClip,
    ) -> Result<(), String> {
        let method = if clip.output_format.eq_ignore_ascii_case("opus") {
            "sendVoice"
        } else {
            "sendAudio"
        };
        let file_field = if method == "sendVoice" {
            "voice"
        } else {
            "audio"
        };
        let format_normalized = clip.output_format.trim().to_ascii_lowercase();
        let (mime, extension) = match format_normalized.as_str() {
            "wav" | "wave" => ("audio/wav", "wav"),
            "opus" | "ogg" | "oga" => ("audio/ogg", "ogg"),
            _ => ("audio/mpeg", "mp3"),
        };
        let mut fields = vec![("chat_id", chat_id.to_string())];
        if let Some(reply_to) = reply_to_message_id {
            fields.push(("reply_to_message_id", reply_to.to_string()));
        }
        if let Some(caption_text) = caption.and_then(normalize_optional_text) {
            fields.push(("caption", truncate_text(&caption_text, 1_000)));
        } else if !clip.real_audio {
            fields.push((
                "caption",
                "OpenClaw TTS fallback audio (simulated voice source)".to_owned(),
            ));
        }
        if let Some(duration_ms) = clip.duration_ms {
            fields.push(("duration", (duration_ms / 1_000).to_string()));
        }
        if method == "sendAudio" {
            if let Some(provider_used) = clip.provider_used.as_ref() {
                fields.push((
                    "title",
                    truncate_text(&format!("OpenClaw TTS ({provider_used})"), 64),
                ));
            } else {
                fields.push(("title", "OpenClaw TTS".to_owned()));
            }
        }
        let file_name = format!("openclaw-tts-{file_field}.{extension}");
        let _ = self
            .telegram_api_multipart(
                token, method, &fields, file_field, &file_name, mime, clip.bytes,
            )
            .await?;
        Ok(())
    }
}

async fn load_offset(primary: &Path, legacy: &Path) -> Result<u64, String> {
    for path in [primary, legacy] {
        if !path.exists() {
            continue;
        }
        let raw = fs::read_to_string(path)
            .await
            .map_err(|err| format!("failed reading offset file {}: {err}", path.display()))?;
        let value: Value = serde_json::from_str(&raw)
            .map_err(|err| format!("failed parsing offset file {}: {err}", path.display()))?;
        if let Some(offset) = value.get("offset").and_then(Value::as_u64) {
            return Ok(offset);
        }
    }
    Ok(0)
}

async fn read_response_frame(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    response_id: &str,
    timeout: Duration,
) -> Result<Value, String> {
    let result = tokio::time::timeout(timeout, async {
        loop {
            let Some(message) = ws.next().await else {
                return Err("gateway socket closed before response".to_owned());
            };
            let message = message.map_err(|err| format!("gateway websocket read failed: {err}"))?;
            match message {
                Message::Text(text) => {
                    let parsed: Value = serde_json::from_str(&text)
                        .map_err(|err| format!("gateway frame JSON parse failed: {err}"))?;
                    if parsed.get("id").and_then(Value::as_str) == Some(response_id) {
                        return Ok(parsed);
                    }
                }
                Message::Ping(payload) => {
                    ws.send(Message::Pong(payload))
                        .await
                        .map_err(|err| format!("gateway websocket pong failed: {err}"))?;
                }
                Message::Close(frame) => {
                    let reason = frame
                        .as_ref()
                        .map(|value| value.reason.to_string())
                        .unwrap_or_else(|| "close".to_owned());
                    return Err(format!("gateway socket closed: {reason}"));
                }
                Message::Binary(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    })
    .await;
    match result {
        Ok(value) => value,
        Err(_) => Err(format!(
            "gateway request timed out waiting for id {response_id}"
        )),
    }
}

fn derive_gateway_ws_url(gateway: &GatewayConfig) -> String {
    let bind = gateway.server.bind.trim();
    if let Ok(socket) = bind.parse::<SocketAddr>() {
        return format!("ws://127.0.0.1:{}/ws", socket.port());
    }
    if let Some((_, port)) = bind.rsplit_once(':') {
        let port = port.trim().trim_end_matches(']');
        if !port.is_empty() && port.chars().all(|ch| ch.is_ascii_digit()) {
            return format!("ws://127.0.0.1:{port}/ws");
        }
    }
    let url = gateway.url.trim();
    if url.is_empty() {
        "ws://127.0.0.1:18789/ws".to_owned()
    } else {
        url.to_owned()
    }
}

fn derive_offset_path(session_state_path: &Path) -> PathBuf {
    let root = session_state_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".openclaw-rs"));
    root.join("telegram").join(TELEGRAM_OFFSET_FILE_NAME)
}

fn normalize_optional_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn extract_telegram_settings(config: &Value) -> Option<TelegramSettings> {
    let channels = config.get("channels")?.as_object()?;
    let telegram = channels.get("telegram")?.as_object()?;
    let bot_token = telegram
        .get("botToken")
        .and_then(Value::as_str)
        .and_then(normalize_optional_text)?;

    let dm_policy = telegram
        .get("dmPolicy")
        .and_then(Value::as_str)
        .and_then(normalize_optional_text)
        .unwrap_or_else(|| "allowlist".to_owned())
        .to_ascii_lowercase();
    let group_policy = telegram
        .get("groupPolicy")
        .and_then(Value::as_str)
        .and_then(normalize_optional_text)
        .unwrap_or_else(|| "mention".to_owned())
        .to_ascii_lowercase();
    let mut allow_from = telegram
        .get("allowFrom")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .filter_map(normalize_optional_text)
                .map(|value| value.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    allow_from.sort();
    allow_from.dedup();

    let candidates = extract_model_candidates(config);
    Some(TelegramSettings {
        bot_token,
        dm_policy,
        group_policy,
        allow_from,
        candidates,
    })
}

fn extract_model_candidates(config: &Value) -> Vec<ModelCandidate> {
    let Some(providers) = config
        .pointer("/models/providers")
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };
    let mut weighted = Vec::new();
    for (provider_name, entry) in providers {
        let normalized_provider = normalize_provider_alias(provider_name);
        let Some(models) = entry.get("models").and_then(Value::as_array) else {
            continue;
        };
        for (index, model_entry) in models.iter().enumerate() {
            let Some(model_id) = model_entry
                .get("id")
                .or_else(|| model_entry.get("model"))
                .and_then(Value::as_str)
                .and_then(normalize_optional_text)
            else {
                continue;
            };
            weighted.push((
                provider_priority(&normalized_provider),
                normalized_provider.clone(),
                index,
                model_id,
            ));
        }
    }
    weighted.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| left.3.cmp(&right.3))
    });

    let mut dedup = HashSet::new();
    let mut out = Vec::new();
    for (_, provider, _, model) in weighted {
        let key = format!("{provider}/{model}");
        if !dedup.insert(key) {
            continue;
        }
        out.push(ModelCandidate { provider, model });
    }
    out
}

fn provider_priority(provider: &str) -> usize {
    match provider {
        "opencode" => 0,
        "openrouter" => 2,
        "qwen-portal" => 3,
        "zai" => 4,
        "zhipuai" => 4,
        "inception" => 5,
        "openai" => 6,
        "anthropic" => 7,
        _ => 100,
    }
}

fn normalize_provider_alias(provider: &str) -> String {
    let normalized = provider.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "z.ai" | "z-ai" | "zaiweb" | "zai-web" => "zai".to_owned(),
        "zhipu-coding" | "zhipuai-coding" | "bigmodel-coding" => "zhipuai-coding".to_owned(),
        "zhipu" | "zhipu-ai" | "zhipuai" | "bigmodel" | "bigmodel-cn" => "zhipuai".to_owned(),
        "opencode-zen" | "opencodefree" | "opencode_free" | "opencode-free" | "opencode-go" => {
            "opencode".to_owned()
        }
        "qwen" | "qwen3.5" | "qwen-3.5" | "qwen35" | "qwen-chat" => "qwen-portal".to_owned(),
        "inception-labs" | "inceptionlabs" | "mercury" | "mercury2" | "mercury-2" => {
            "inception".to_owned()
        }
        "kimi-code" | "kimi-for-coding" => "kimi-coding".to_owned(),
        "gemini" | "google-gemini-cli" => "google".to_owned(),
        "bytedance" | "doubao" => "volcengine".to_owned(),
        "fireworks-ai" => "fireworks".to_owned(),
        "moonshotai" | "moonshotai-cn" => "moonshot".to_owned(),
        "novita-ai" => "novita".to_owned(),
        "inference" => "inference-net".to_owned(),
        "chatgpt" => "openai".to_owned(),
        "codex" | "codex-cli" => "openai-codex".to_owned(),
        "claude" | "claude-cli" | "claude-code" => "anthropic".to_owned(),
        value => value.to_owned(),
    }
}

fn command_token_without_mention(token: &str) -> String {
    let lowered = token.trim().to_ascii_lowercase();
    if let Some((prefix, _)) = lowered.split_once('@') {
        prefix.to_owned()
    } else {
        lowered
    }
}

fn parse_telegram_control_command(text: &str) -> Option<TelegramControlCommand> {
    let trimmed = text.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let mut parts = trimmed.split_whitespace();
    let command = command_token_without_mention(parts.next()?);
    let raw_args = parts.collect::<Vec<_>>().join(" ");
    match command.as_str() {
        "/model" => Some(TelegramControlCommand::Model { raw_args }),
        "/set" => Some(TelegramControlCommand::SetApiKey { raw_args }),
        "/auth" => Some(TelegramControlCommand::Auth { raw_args }),
        "/tts" => Some(TelegramControlCommand::Tts { raw_args }),
        _ => None,
    }
}

fn resolve_model_selection(
    catalog: &[CatalogModel],
    args: &[&str],
) -> Result<(String, String, bool), String> {
    if args.is_empty() {
        return Err("model command requires arguments".to_owned());
    }
    let (provider_raw, model_raw) = if args.len() == 1 {
        if let Some((provider, model)) = args[0].split_once('/') {
            (provider, Some(model.to_owned()))
        } else {
            (args[0], None)
        }
    } else {
        (args[0], Some(args[1..].join(" ")))
    };
    let normalized_provider = normalize_provider_alias(provider_raw);
    let Some(provider) = normalize_optional_text(&normalized_provider) else {
        return Err("provider is required".to_owned());
    };
    let provider_catalog = catalog
        .iter()
        .filter(|entry| entry.provider.eq_ignore_ascii_case(&provider))
        .collect::<Vec<_>>();
    if let Some(requested_raw) = model_raw {
        let Some(requested_model) = normalize_optional_text(&requested_raw) else {
            return Err("model id is required".to_owned());
        };
        if let Some(entry) = provider_catalog
            .iter()
            .find(|entry| {
                entry.id.eq_ignore_ascii_case(&requested_model)
                    || entry.name.eq_ignore_ascii_case(&requested_model)
            })
            .copied()
        {
            return Ok((provider, entry.id.clone(), true));
        }
        return Ok((provider, requested_model, false));
    }
    let Some(default_entry) = provider_catalog.first() else {
        return Err(format!(
            "provider `{provider}` has no catalog models. Use `/model list` first."
        ));
    };
    Ok((provider, default_entry.id.clone(), true))
}

fn format_model_help(catalog: &[CatalogModel]) -> String {
    let mut by_provider: BTreeMap<String, usize> = BTreeMap::new();
    for entry in catalog {
        *by_provider.entry(entry.provider.clone()).or_insert(0) += 1;
    }
    let mut out = String::new();
    out.push_str("Model command usage:\n");
    out.push_str("/model list\n");
    out.push_str("/model list <provider>\n");
    out.push_str("/model <provider>/<model>\n");
    out.push_str("/model <provider> <model>\n");
    out.push_str("/set api key <provider> <key>\n");
    out.push_str("/auth providers\n");
    out.push_str("/auth start <provider> [account]\n");
    out.push_str("/auth wait <provider> [session_id]\n");
    out.push_str("/tts status\n");
    out.push_str("/tts speak <text>\n");
    if !by_provider.is_empty() {
        out.push_str("\nProviders in catalog:\n");
        for (index, (provider, count)) in by_provider.iter().enumerate() {
            if index >= TELEGRAM_MODEL_HELP_MAX_MODELS {
                let remaining = by_provider
                    .len()
                    .saturating_sub(TELEGRAM_MODEL_HELP_MAX_MODELS);
                let _ = writeln!(out, "... and {remaining} more");
                break;
            }
            let _ = writeln!(out, "- {provider} ({count})");
        }
    }
    out.trim_end().to_owned()
}

fn format_model_list(catalog: &[CatalogModel], provider_filter: Option<&str>) -> String {
    if let Some(provider_raw) = provider_filter {
        let provider = normalize_provider_alias(provider_raw);
        let models = catalog
            .iter()
            .filter(|entry| entry.provider.eq_ignore_ascii_case(&provider))
            .collect::<Vec<_>>();
        if models.is_empty() {
            return format!("No models found for provider `{provider}`.");
        }
        let mut out = format!("Models for `{provider}`:\n");
        for (index, entry) in models.iter().enumerate() {
            if index >= TELEGRAM_MODEL_LIST_MAX_MODELS {
                let remaining = models.len().saturating_sub(TELEGRAM_MODEL_LIST_MAX_MODELS);
                let _ = writeln!(out, "... and {remaining} more");
                break;
            }
            if entry.name.eq_ignore_ascii_case(&entry.id) {
                let _ = writeln!(out, "- {}", entry.id);
            } else {
                let _ = writeln!(out, "- {} ({})", entry.id, entry.name);
            }
        }
        out.push_str("Use: /model ");
        out.push_str(&provider);
        out.push_str("/<model>");
        return out.trim_end().to_owned();
    }

    let mut grouped: BTreeMap<String, Vec<&CatalogModel>> = BTreeMap::new();
    for entry in catalog {
        grouped
            .entry(entry.provider.clone())
            .or_default()
            .push(entry);
    }
    if grouped.is_empty() {
        return "No models are currently available.".to_owned();
    }
    let mut out = String::from("Providers:\n");
    for (index, (provider, models)) in grouped.iter().enumerate() {
        if index >= TELEGRAM_MODEL_LIST_MAX_MODELS {
            let remaining = grouped.len().saturating_sub(TELEGRAM_MODEL_LIST_MAX_MODELS);
            let _ = writeln!(out, "... and {remaining} more");
            break;
        }
        let sample = models
            .first()
            .map(|entry| entry.id.as_str())
            .unwrap_or("<none>");
        let _ = writeln!(
            out,
            "- {provider}: {} models (example: {sample})",
            models.len()
        );
    }
    out.push_str("Use `/model list <provider>` for full model IDs.");
    out.trim_end().to_owned()
}

fn format_auth_help() -> String {
    let mut out = String::new();
    out.push_str("Auth command usage:\n");
    out.push_str("/auth providers\n");
    out.push_str("/auth status [provider] [account]\n");
    out.push_str("/auth bridge\n");
    out.push_str("/auth start <provider> [account] [--force]\n");
    out.push_str("/auth wait <provider> [session_id] [account] [--timeout <seconds>]\n");
    out.push_str("/auth wait session <session_id> [account]\n");
    out.push_str("/auth help\n");
    out.push_str("\nExamples:\n");
    out.push_str("/auth start kimi\n");
    out.push_str("/auth status openai\n");
    out.push_str("/auth bridge\n");
    out.push_str("/auth wait kimi --timeout 90\n");
    out.push_str("/auth wait session <session_id>\n");
    out.trim_end().to_owned()
}

fn format_tts_help() -> String {
    let mut out = String::new();
    out.push_str("TTS command usage:\n");
    out.push_str("/tts status\n");
    out.push_str("/tts providers\n");
    out.push_str("/tts provider <openai|elevenlabs|kittentts|edge>\n");
    out.push_str("/tts on\n");
    out.push_str("/tts off\n");
    out.push_str("/tts speak <text>\n");
    out.push_str("/tts help\n");
    out.push_str("\nNotes:\n");
    out.push_str("- `on/off` toggles runtime TTS.\n");
    out.push_str("- `speak` sends an audio clip directly in Telegram.\n");
    out.trim_end().to_owned()
}

fn format_auth_provider_list(result: &Value) -> String {
    let providers = result
        .get("providers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if providers.is_empty() {
        return "No OAuth providers are currently configured.".to_owned();
    }

    let mut out = String::from("OAuth providers:\n");
    for (index, entry) in providers.iter().enumerate() {
        if index >= TELEGRAM_AUTH_LIST_MAX_PROVIDERS {
            let remaining = providers
                .len()
                .saturating_sub(TELEGRAM_AUTH_LIST_MAX_PROVIDERS);
            let _ = writeln!(out, "... and {remaining} more");
            break;
        }
        let provider_id = entry
            .get("providerId")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text)
            .unwrap_or_else(|| "<unknown>".to_owned());
        let display_name = entry
            .get("displayName")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text)
            .unwrap_or_else(|| provider_id.clone());
        let connected_accounts = entry
            .get("connectedAccounts")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if display_name.eq_ignore_ascii_case(&provider_id) {
            let _ = writeln!(out, "- {provider_id} (connected: {connected_accounts})");
        } else {
            let _ = writeln!(
                out,
                "- {provider_id} ({display_name}, connected: {connected_accounts})"
            );
        }

        let aliases = entry
            .get("aliases")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .filter_map(normalize_optional_text)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !aliases.is_empty() {
            let _ = writeln!(out, "  aliases: {}", aliases.join(", "));
        }
        if let Some(url) = entry
            .get("verificationUrl")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text)
        {
            let _ = writeln!(out, "  verify: {url}");
        }
    }
    out.push_str("Use `/auth start <provider>` to begin OAuth login.");
    out.trim_end().to_owned()
}

fn parse_auth_wait_timeout_ms(args: &[&str]) -> u64 {
    let mut idx = 0usize;
    while idx < args.len() {
        let token = args[idx].trim();
        let mut parsed: Option<u64> = None;
        if let Some(value) = token.strip_prefix("--timeout-ms=") {
            parsed = value.parse::<u64>().ok();
        } else if token.eq_ignore_ascii_case("--timeout-ms") {
            parsed = args
                .get(idx + 1)
                .and_then(|value| value.parse::<u64>().ok());
        } else if let Some(value) = token.strip_prefix("--timeout=") {
            parsed = value
                .parse::<u64>()
                .ok()
                .map(|seconds| seconds.saturating_mul(1_000));
        } else if token.eq_ignore_ascii_case("--timeout") {
            parsed = args
                .get(idx + 1)
                .and_then(|value| value.parse::<u64>().ok())
                .map(|seconds| seconds.saturating_mul(1_000));
        }
        if let Some(timeout_ms) = parsed {
            return timeout_ms.clamp(
                TELEGRAM_AUTH_WAIT_MIN_TIMEOUT_MS,
                TELEGRAM_AUTH_WAIT_MAX_TIMEOUT_MS,
            );
        }
        idx = idx.saturating_add(1);
    }
    TELEGRAM_AUTH_WAIT_DEFAULT_TIMEOUT_MS
}

fn to_bridge_health_url(raw: &str) -> Option<String> {
    let candidate = normalize_optional_text(raw)?;
    if candidate.contains("/health") {
        return Some(candidate);
    }
    if let Some(prefix) = candidate.strip_suffix("/v1/chat/completions") {
        return Some(format!("{}{}", prefix.trim_end_matches('/'), "/health"));
    }
    if let Some(prefix) = candidate.strip_suffix("/v1") {
        return Some(format!("{}{}", prefix.trim_end_matches('/'), "/health"));
    }
    Some(format!("{}/health", candidate.trim_end_matches('/')))
}

fn parse_tts_audio_clip(payload: &Value) -> Result<TelegramTtsAudioClip, String> {
    let audio_base64 = payload
        .get("audioBase64")
        .and_then(Value::as_str)
        .and_then(normalize_optional_text)
        .ok_or_else(|| "tts.convert response missing audioBase64".to_owned())?;
    let bytes = BASE64_STANDARD
        .decode(audio_base64.as_bytes())
        .map_err(|err| format!("tts.convert audio decode failed: {err}"))?;
    if bytes.is_empty() {
        return Err("tts.convert returned empty audio bytes".to_owned());
    }
    let output_format = payload
        .get("outputFormat")
        .and_then(Value::as_str)
        .and_then(normalize_optional_text)
        .unwrap_or_else(|| "mp3".to_owned());
    let duration_ms = payload.get("durationMs").and_then(Value::as_u64);
    let provider_used = payload
        .get("providerUsed")
        .and_then(Value::as_str)
        .and_then(normalize_optional_text);
    let source = payload
        .get("synthSource")
        .and_then(Value::as_str)
        .and_then(normalize_optional_text);
    let real_audio = source
        .as_deref()
        .map(|value| !value.eq_ignore_ascii_case("simulated"))
        .unwrap_or(false);
    Ok(TelegramTtsAudioClip {
        bytes,
        output_format,
        duration_ms,
        provider_used,
        source,
        real_audio,
    })
}

fn extract_message_text(message: &Value) -> Option<String> {
    message
        .get("text")
        .and_then(Value::as_str)
        .and_then(normalize_optional_text)
        .or_else(|| {
            message
                .get("caption")
                .and_then(Value::as_str)
                .and_then(normalize_optional_text)
        })
}

fn is_help_command(text: &str) -> bool {
    let lowered = text.trim().to_ascii_lowercase();
    lowered == "/start" || lowered == "/help"
}

fn is_allowed_by_dm_policy(message: &Value, dm_policy: &str, allow_from: &[String]) -> bool {
    if !dm_policy.eq_ignore_ascii_case("allowlist") {
        return true;
    }
    if allow_from.is_empty() {
        return false;
    }
    let mut tags = HashSet::new();
    if let Some(from) = message.get("from").and_then(Value::as_object) {
        if let Some(id) = from.get("id").and_then(Value::as_i64) {
            tags.insert(id.to_string());
            tags.insert(format!("telegram:{id}"));
        }
        if let Some(username) = from
            .get("username")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text)
            .map(|value| value.to_ascii_lowercase())
        {
            let user = username.trim_start_matches('@').to_owned();
            tags.insert(user.clone());
            tags.insert(format!("@{user}"));
            tags.insert(format!("telegram:@{user}"));
        }
    }
    if let Some(chat_id) = message
        .get("chat")
        .and_then(|value| value.get("id"))
        .and_then(Value::as_i64)
    {
        tags.insert(chat_id.to_string());
        tags.insert(format!("telegram-chat:{chat_id}"));
    }
    allow_from
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .any(|value| tags.contains(&value))
}

fn allows_group_message(
    message: &Value,
    chat_type: &str,
    group_policy: &str,
    bot_id: i64,
    bot_username: Option<&str>,
) -> bool {
    if chat_type.eq_ignore_ascii_case("private") {
        return true;
    }
    if matches!(group_policy, "all" | "always" | "any") {
        return true;
    }
    if let Some(reply_to_bot) = message
        .get("reply_to_message")
        .and_then(|value| value.get("from"))
        .and_then(|value| value.get("id"))
        .and_then(Value::as_i64)
        .map(|id| id == bot_id)
    {
        if reply_to_bot {
            return true;
        }
    }

    let Some(text) = extract_message_text(message) else {
        return false;
    };
    if is_help_command(&text) {
        return true;
    }
    let Some(username) = bot_username.map(|value| value.trim().trim_start_matches('@')) else {
        return false;
    };
    if username.is_empty() {
        return false;
    }
    let needle = format!("@{}", username.to_ascii_lowercase());
    text.to_ascii_lowercase().contains(&needle)
}

fn build_session_key(message: &Value) -> String {
    let chat = message.get("chat").and_then(Value::as_object);
    let chat_type = chat
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("private");
    let chat_id = chat
        .and_then(|value| value.get("id"))
        .and_then(Value::as_i64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_owned());
    let from_id = message
        .get("from")
        .and_then(|value| value.get("id"))
        .and_then(Value::as_i64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_owned());
    let topic = message
        .get("message_thread_id")
        .and_then(Value::as_i64)
        .map(|value| value.to_string());

    if chat_type.eq_ignore_ascii_case("private") {
        return format!("agent:main:telegram:dm:{from_id}");
    }
    if let Some(topic_id) = topic {
        return format!("agent:main:telegram:group:{chat_id}:topic:{topic_id}");
    }
    format!("agent:main:telegram:group:{chat_id}")
}

fn extract_assistant_reply(history_result: &Value, run_id: &str) -> Option<String> {
    let entries = history_result.get("history").and_then(Value::as_array)?;
    for entry in entries.iter().rev() {
        let source = entry
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !source.eq_ignore_ascii_case("agent.assistant") {
            continue;
        }
        let request_id = entry
            .get("requestId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if request_id != run_id {
            continue;
        }
        if let Some(text) = entry
            .get("text")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text)
        {
            return Some(text);
        }
    }
    for entry in entries.iter().rev() {
        let source = entry
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !source.eq_ignore_ascii_case("agent.assistant") {
            continue;
        }
        if let Some(text) = entry
            .get("text")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text)
        {
            return Some(text);
        }
    }
    None
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut count = 0usize;
    for ch in value.chars() {
        if count >= max_chars {
            break;
        }
        out.push(ch);
        count = count.saturating_add(1);
    }
    if value.chars().count() > max_chars && max_chars > 1 {
        let mut trimmed = out
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect::<String>();
        trimmed.push('…');
        trimmed
    } else {
        out
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_millis() as u64
}

impl TelegramBridge {
    async fn patch_session_model_override(
        &self,
        session_key: &str,
        model_override: Option<&str>,
    ) -> Result<(), String> {
        let model_value = match model_override.and_then(normalize_optional_text) {
            Some(value) => Value::String(value),
            None => Value::Null,
        };
        self.gateway_rpc_call(
            "sessions.patch",
            json!({
                "sessionKey": session_key,
                "model": model_value
            }),
            Duration::from_secs(15),
            &["operator.admin"],
        )
        .await?;
        Ok(())
    }

    async fn read_session_model_override(&self, session_key: &str) -> Option<String> {
        let status = self
            .gateway_rpc_call(
                "session.status",
                json!({
                    "sessionKey": session_key
                }),
                Duration::from_secs(15),
                &["operator.admin"],
            )
            .await
            .ok()?;
        status
            .pointer("/session/modelOverride")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text)
    }

    async fn process_update(
        &self,
        settings: &TelegramSettings,
        bot_identity: &TelegramBotIdentity,
        update: &Value,
        update_id: u64,
    ) -> Result<(), String> {
        let Some(message) = update.get("message") else {
            return Ok(());
        };
        if !message.is_object() {
            return Ok(());
        }

        let from = message.get("from");
        if from
            .and_then(|value| value.get("is_bot"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(());
        }
        if from
            .and_then(|value| value.get("id"))
            .and_then(Value::as_i64)
            == Some(bot_identity.id)
        {
            return Ok(());
        }

        let chat_id = message
            .get("chat")
            .and_then(|value| value.get("id"))
            .and_then(Value::as_i64)
            .ok_or_else(|| "telegram message missing chat.id".to_owned())?;
        let chat_type = message
            .get("chat")
            .and_then(|value| value.get("type"))
            .and_then(Value::as_str)
            .and_then(normalize_optional_text)
            .unwrap_or_else(|| "private".to_owned());
        let message_id = message.get("message_id").and_then(Value::as_i64);
        let Some(text) = extract_message_text(message) else {
            return Ok(());
        };

        if !is_allowed_by_dm_policy(message, &settings.dm_policy, &settings.allow_from) {
            debug!(
                "telegram update {} blocked by dmPolicy allowlist",
                update_id
            );
            return Ok(());
        }
        if !allows_group_message(
            message,
            &chat_type,
            settings.group_policy.as_str(),
            bot_identity.id,
            bot_identity.username.as_deref(),
        ) {
            return Ok(());
        }

        if let Err(err) = self.emit_inbound_event(update_id).await {
            debug!("telegram inbound event skipped: {err}");
        }

        let session_key = build_session_key(message);
        if is_help_command(&text) {
            self.send_message(
                &settings.bot_token,
                chat_id,
                "OpenClaw Rust is online. Send a message and I will respond.",
                message_id,
            )
            .await?;
            if let Err(err) = self.emit_outbound_event(update_id).await {
                debug!("telegram outbound event skipped: {err}");
            }
            return Ok(());
        }
        if let Some(command) = parse_telegram_control_command(&text) {
            let response = self
                .handle_control_command(
                    &session_key,
                    command,
                    &settings.bot_token,
                    chat_id,
                    message_id,
                )
                .await
                .unwrap_or_else(|err| {
                    format!("OpenClaw Rust command error: {}", truncate_text(&err, 350))
                });
            if !response.is_empty() {
                self.send_message(&settings.bot_token, chat_id, &response, message_id)
                    .await?;
            }
            if let Err(err) = self.emit_outbound_event(update_id).await {
                debug!("telegram outbound event skipped: {err}");
            }
            return Ok(());
        }

        let thread_id = message
            .get("message_thread_id")
            .and_then(Value::as_i64)
            .map(|value| value.to_string());
        let response = self
            .run_agent_with_fallback(&session_key, &text, chat_id, thread_id, settings, update_id)
            .await
            .unwrap_or_else(|err| format!("OpenClaw Rust error: {}", truncate_text(&err, 350)));

        self.send_message(&settings.bot_token, chat_id, &response, message_id)
            .await?;
        if let Err(err) = self
            .maybe_send_tts_reply(&settings.bot_token, chat_id, message_id, &response)
            .await
        {
            debug!("telegram tts reply skipped: {err}");
        }
        if let Err(err) = self.emit_outbound_event(update_id).await {
            debug!("telegram outbound event skipped: {err}");
        }
        Ok(())
    }

    async fn handle_control_command(
        &self,
        session_key: &str,
        command: TelegramControlCommand,
        token: &str,
        chat_id: i64,
        reply_to_message_id: Option<i64>,
    ) -> Result<String, String> {
        match command {
            TelegramControlCommand::Model { raw_args } => {
                self.handle_model_command(session_key, &raw_args).await
            }
            TelegramControlCommand::SetApiKey { raw_args } => {
                self.handle_set_api_key_command(&raw_args).await
            }
            TelegramControlCommand::Auth { raw_args } => self.handle_auth_command(&raw_args).await,
            TelegramControlCommand::Tts { raw_args } => {
                self.handle_tts_command(&raw_args, token, chat_id, reply_to_message_id)
                    .await
            }
        }
    }

    async fn handle_auth_command(&self, raw_args: &str) -> Result<String, String> {
        let args = raw_args
            .split_whitespace()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if args.is_empty() || args[0].eq_ignore_ascii_case("help") {
            return Ok(format_auth_help());
        }
        if args[0].eq_ignore_ascii_case("providers") || args[0].eq_ignore_ascii_case("list") {
            let result = self
                .gateway_rpc_call(
                    "auth.oauth.providers",
                    json!({}),
                    Duration::from_secs(15),
                    &["operator.read"],
                )
                .await?;
            return Ok(format_auth_provider_list(&result));
        }
        if args[0].eq_ignore_ascii_case("status") {
            return self.handle_auth_status_command(&args).await;
        }
        if args[0].eq_ignore_ascii_case("bridge") {
            return self.handle_auth_bridge_command().await;
        }
        if args[0].eq_ignore_ascii_case("start") {
            return self.handle_auth_start_command(&args).await;
        }
        if args[0].eq_ignore_ascii_case("wait") {
            return self.handle_auth_wait_command(&args).await;
        }
        Ok(format!(
            "Unknown /auth subcommand `{}`.\n\n{}",
            args[0],
            format_auth_help()
        ))
    }

    async fn handle_auth_status_command(&self, args: &[&str]) -> Result<String, String> {
        let requested_provider = args
            .get(1)
            .map(|value| normalize_provider_alias(value))
            .and_then(|value| normalize_optional_text(&value))
            .unwrap_or_else(|| "openai".to_owned());
        let requested_account = args
            .get(2)
            .and_then(|value| normalize_optional_text(value))
            .unwrap_or_else(|| TELEGRAM_ACCOUNT_ID.to_owned());
        let providers = self
            .gateway_rpc_call(
                "auth.oauth.providers",
                json!({}),
                Duration::from_secs(15),
                &["operator.read"],
            )
            .await?;
        let entries = providers
            .get("providers")
            .and_then(Value::as_array)
            .ok_or_else(|| "auth.oauth.providers missing providers".to_owned())?;
        let Some(provider_entry) = entries.iter().find(|entry| {
            entry
                .get("providerId")
                .and_then(Value::as_str)
                .map(|value| value.eq_ignore_ascii_case(&requested_provider))
                .unwrap_or(false)
        }) else {
            return Ok(format!(
                "OAuth provider `{requested_provider}` not found. Use `/auth providers`."
            ));
        };

        let display_name = provider_entry
            .get("displayName")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text)
            .unwrap_or_else(|| requested_provider.clone());
        let connected_accounts = provider_entry
            .get("connectedAccounts")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let mut out = format!(
            "OAuth status for `{requested_provider}` ({display_name})\nConnected accounts: {connected_accounts}"
        );
        let accounts = provider_entry
            .get("accounts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if accounts.is_empty() {
            let _ = writeln!(out, "\nNo accounts connected.");
            if requested_provider.eq_ignore_ascii_case("openai") {
                let _ = writeln!(out, "Start with: /auth start openai");
            }
            return Ok(out.trim_end().to_owned());
        }
        for account in accounts.iter().take(8) {
            let account_id = account
                .get("accountId")
                .and_then(Value::as_str)
                .and_then(normalize_optional_text)
                .unwrap_or_else(|| "default".to_owned());
            let connected_marker = if account_id.eq_ignore_ascii_case(&requested_account) {
                "*"
            } else {
                "-"
            };
            let profile_id = account
                .get("profileId")
                .and_then(Value::as_str)
                .and_then(normalize_optional_text)
                .unwrap_or_else(|| "<unknown>".to_owned());
            let expires_at_ms = account.get("expiresAtMs").and_then(Value::as_u64);
            let source = account
                .get("source")
                .and_then(Value::as_str)
                .and_then(normalize_optional_text)
                .unwrap_or_else(|| "unknown".to_owned());
            let _ = writeln!(
                out,
                "\n{connected_marker} account `{account_id}` profile `{profile_id}`"
            );
            if let Some(expires) = expires_at_ms {
                let _ = writeln!(out, "  expiresAtMs: {expires}");
            }
            let _ = writeln!(out, "  source: {source}");
        }
        if requested_provider.eq_ignore_ascii_case("openai") {
            let _ = writeln!(
                out,
                "\nUse `/auth bridge` to check ChatGPT bridge reachability."
            );
        }
        Ok(out.trim_end().to_owned())
    }

    async fn handle_auth_bridge_command(&self) -> Result<String, String> {
        let result = self
            .gateway_rpc_call(
                "config.get",
                json!({}),
                Duration::from_secs(15),
                &["operator.read"],
            )
            .await?;
        let config = result
            .get("config")
            .cloned()
            .ok_or_else(|| "config.get missing config object".to_owned())?;
        if !config.is_object() {
            return Err("config.get missing config object".to_owned());
        }
        let mut candidates = Vec::new();
        if let Some(values) = config
            .pointer("/auth/oauth/chatgptBrowser/bridgeBaseUrls")
            .and_then(Value::as_array)
        {
            for value in values {
                if let Some(raw) = value.as_str().and_then(normalize_optional_text) {
                    candidates.push(raw);
                }
            }
        }
        if let Some(raw) = config
            .pointer("/models/providers/openai/baseUrl")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text)
        {
            candidates.push(raw);
        }
        if let Some(values) = config
            .pointer("/models/providers/openai/bridgeBaseUrls")
            .and_then(Value::as_array)
        {
            for value in values {
                if let Some(raw) = value.as_str().and_then(normalize_optional_text) {
                    candidates.push(raw);
                }
            }
        }
        candidates.sort();
        candidates.dedup();
        if candidates.is_empty() {
            return Ok(
                "No ChatGPT bridge candidates configured. Set auth/oauth chatgptBrowser bridgeBaseUrls."
                    .to_owned(),
            );
        }
        let mut out = String::from("Auth bridge diagnostics:\n");
        for candidate in candidates.iter().take(TELEGRAM_AUTH_BRIDGE_MAX_CANDIDATES) {
            let Some(health_url) = to_bridge_health_url(candidate) else {
                continue;
            };
            let check = self
                .http
                .get(&health_url)
                .timeout(Duration::from_secs(7))
                .send()
                .await;
            match check {
                Ok(response) => {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    let body_preview = truncate_text(&body.replace('\n', " "), 120);
                    let _ = writeln!(
                        out,
                        "- {candidate}\n  health: {} {}\n  body: {}",
                        status.as_u16(),
                        status.canonical_reason().unwrap_or(""),
                        body_preview
                    );
                }
                Err(err) => {
                    let _ = writeln!(out, "- {candidate}\n  health: error ({})", err);
                }
            }
        }
        out.push_str("If all health checks fail, ensure your local bridge and reverse SSH tunnel are running.");
        Ok(out.trim_end().to_owned())
    }

    async fn handle_auth_start_command(&self, args: &[&str]) -> Result<String, String> {
        if args.len() < 2 {
            return Ok(
                "Usage: /auth start <provider> [account] [--force]\nExample: /auth start kimi"
                    .to_owned(),
            );
        }
        let provider = normalize_provider_alias(args[1]);
        let mut account_id = TELEGRAM_ACCOUNT_ID.to_owned();
        let mut force = false;
        for token in args.iter().skip(2) {
            if token.eq_ignore_ascii_case("--force") {
                force = true;
                continue;
            }
            if account_id == TELEGRAM_ACCOUNT_ID {
                if let Some(account) = normalize_optional_text(token) {
                    account_id = account;
                }
            }
        }

        let result = self
            .gateway_rpc_call(
                "auth.oauth.start",
                json!({
                    "provider": provider.clone(),
                    "accountId": account_id.clone(),
                    "timeoutMs": 300_000,
                    "force": force
                }),
                Duration::from_secs(20),
                &["operator.read", "operator.write"],
            )
            .await?;
        let resolved_provider = result
            .get("providerId")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text)
            .unwrap_or_else(|| provider.clone());
        let resolved_account = result
            .get("accountId")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text)
            .unwrap_or_else(|| account_id.clone());
        let session_id = result
            .get("sessionId")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text);
        let verification_url = result
            .get("verificationUrl")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text);
        let user_code = result
            .get("userCode")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text);
        let poll_interval_ms = result.get("pollIntervalMs").and_then(Value::as_u64);
        let expires_at_ms = result.get("expiresAtMs").and_then(Value::as_u64);
        let message = result
            .get("message")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text);

        let mut out = format!(
            "OAuth login started for `{resolved_provider}` (account `{resolved_account}`)."
        );
        if let Some(url) = verification_url {
            let _ = writeln!(out, "\nOpen: {url}");
        }
        if let Some(code) = user_code {
            let _ = writeln!(out, "Code: {code}");
        }
        if let Some(session) = &session_id {
            let _ = writeln!(out, "Session: {session}");
        }
        if let Some(interval) = poll_interval_ms {
            let _ = writeln!(out, "Suggested wait poll interval: {}s", interval / 1_000);
        }
        if let Some(expires_at) = expires_at_ms {
            let _ = writeln!(out, "ExpiresAtMs: {expires_at}");
        }
        if let Some(value) = message {
            let _ = writeln!(out, "{value}");
        }
        if resolved_provider.eq_ignore_ascii_case("openai") {
            let _ = writeln!(
                out,
                "OpenAI browser auth note: if you do not see a code field, just complete ChatGPT login in the bridge browser session."
            );
        }
        if let Some(session) = session_id {
            let _ = write!(
                out,
                "Run `/auth wait {resolved_provider} {session}` after login."
            );
        } else {
            let _ = write!(out, "Run `/auth wait {resolved_provider}` after login.");
        }
        Ok(out.trim_end().to_owned())
    }

    async fn handle_auth_wait_command(&self, args: &[&str]) -> Result<String, String> {
        if args.len() < 2 {
            return Ok(
                "Usage: /auth wait <provider> [session_id] [account]\nOr: /auth wait session <session_id> [account]"
                    .to_owned(),
            );
        }
        let timeout_ms = parse_auth_wait_timeout_ms(args);
        let mut provider: Option<String> = None;
        let mut session_id: Option<String> = None;
        let mut account_id = TELEGRAM_ACCOUNT_ID.to_owned();

        if args[1].eq_ignore_ascii_case("session") {
            if args.len() < 3 {
                return Ok("Usage: /auth wait session <session_id> [account]".to_owned());
            }
            session_id = normalize_optional_text(args[2]);
            let mut idx = 3usize;
            while idx < args.len() {
                let token = args[idx];
                if token.eq_ignore_ascii_case("--timeout")
                    || token.eq_ignore_ascii_case("--timeout-ms")
                {
                    idx = idx.saturating_add(2);
                    continue;
                }
                if token.starts_with("--timeout=") || token.starts_with("--timeout-ms=") {
                    idx = idx.saturating_add(1);
                    continue;
                }
                if let Some(account) = normalize_optional_text(token) {
                    account_id = account;
                    break;
                }
                idx = idx.saturating_add(1);
            }
        } else {
            let normalized_provider = normalize_provider_alias(args[1]);
            provider = normalize_optional_text(&normalized_provider);
            let mut positional = Vec::new();
            let mut idx = 2usize;
            while idx < args.len() {
                let token = args[idx];
                if token.eq_ignore_ascii_case("--timeout")
                    || token.eq_ignore_ascii_case("--timeout-ms")
                {
                    idx = idx.saturating_add(2);
                    continue;
                }
                if token.starts_with("--timeout=") || token.starts_with("--timeout-ms=") {
                    idx = idx.saturating_add(1);
                    continue;
                }
                positional.push(token);
                idx = idx.saturating_add(1);
            }
            if let Some(value) = positional
                .first()
                .and_then(|value| normalize_optional_text(value))
            {
                session_id = Some(value);
            }
            if let Some(value) = positional
                .get(1)
                .and_then(|value| normalize_optional_text(value))
            {
                account_id = value;
            }
        }
        if provider.is_none() && session_id.is_none() {
            return Ok(
                "Usage: /auth wait <provider> [session_id] [account]\nOr: /auth wait session <session_id> [account]"
                    .to_owned(),
            );
        }

        let result = self
            .gateway_rpc_call(
                "auth.oauth.wait",
                json!({
                    "provider": provider.clone(),
                    "sessionId": session_id.clone(),
                    "accountId": account_id.clone(),
                    "timeoutMs": timeout_ms
                }),
                Duration::from_millis(timeout_ms.saturating_add(15_000)),
                &["operator.read", "operator.write"],
            )
            .await?;
        let resolved_provider = result
            .get("providerId")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text)
            .or(provider)
            .unwrap_or_else(|| "unknown".to_owned());
        let resolved_account = result
            .get("accountId")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text)
            .unwrap_or_else(|| account_id.clone());
        let resolved_session = result
            .get("sessionId")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text)
            .or(session_id);
        let connected = result
            .get("connected")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let status = result
            .get("status")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text)
            .unwrap_or_else(|| "unknown".to_owned());
        let message = result
            .get("message")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text);
        let retry_after_ms = result.get("retryAfterMs").and_then(Value::as_u64);
        let expires_at_ms = result.get("expiresAtMs").and_then(Value::as_u64);
        let profile_id = result
            .get("profileId")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text);

        let mut out = format!(
            "OAuth wait result for `{resolved_provider}` (account `{resolved_account}`): {status}"
        );
        let _ = writeln!(out, "\nTimeoutMs: {timeout_ms}");
        if let Some(session) = &resolved_session {
            let _ = writeln!(out, "\nSession: {session}");
        }
        let _ = writeln!(out, "Connected: {}", if connected { "yes" } else { "no" });
        if let Some(profile) = profile_id {
            let _ = writeln!(out, "Profile: {profile}");
        }
        if let Some(retry_after) = retry_after_ms {
            let _ = writeln!(out, "RetryAfterMs: {retry_after}");
        }
        if let Some(expires_at) = expires_at_ms {
            let _ = writeln!(out, "ExpiresAtMs: {expires_at}");
        }
        if let Some(value) = message {
            let _ = writeln!(out, "{value}");
        }
        if !connected {
            if let Some(session) = resolved_session {
                let _ = write!(
                    out,
                    "If login is still pending, run `/auth wait {resolved_provider} {session}`."
                );
            } else {
                let _ = write!(
                    out,
                    "If login is still pending, run `/auth wait {resolved_provider}`."
                );
            }
        }
        Ok(out.trim_end().to_owned())
    }

    async fn maybe_send_tts_reply(
        &self,
        token: &str,
        chat_id: i64,
        reply_to_message_id: Option<i64>,
        text: &str,
    ) -> Result<(), String> {
        let status = self
            .gateway_rpc_call(
                "tts.status",
                json!({}),
                Duration::from_secs(15),
                &["operator.read"],
            )
            .await?;
        if !status
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(());
        }
        let Some(sample) = normalize_optional_text(text)
            .map(|value| truncate_text(&value, TELEGRAM_TTS_TEXT_MAX_CHARS))
        else {
            return Ok(());
        };
        let convert = self
            .gateway_rpc_call(
                "tts.convert",
                json!({
                    "text": sample,
                    "channel": "telegram",
                    "outputFormat": "wav",
                    "requireRealAudio": false
                }),
                Duration::from_secs(30),
                &["operator.read"],
            )
            .await?;
        let clip = parse_tts_audio_clip(&convert)?;
        self.send_audio_clip(token, chat_id, None, reply_to_message_id, clip)
            .await
    }

    async fn handle_tts_command(
        &self,
        raw_args: &str,
        token: &str,
        chat_id: i64,
        reply_to_message_id: Option<i64>,
    ) -> Result<String, String> {
        let args = raw_args
            .split_whitespace()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if args.is_empty() || args[0].eq_ignore_ascii_case("help") {
            return Ok(format_tts_help());
        }
        if args[0].eq_ignore_ascii_case("status") {
            let status = self
                .gateway_rpc_call(
                    "tts.status",
                    json!({}),
                    Duration::from_secs(15),
                    &["operator.read"],
                )
                .await?;
            let enabled = status
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let provider = status
                .get("provider")
                .and_then(Value::as_str)
                .and_then(normalize_optional_text)
                .unwrap_or_else(|| "unknown".to_owned());
            let fallback = status
                .get("fallbackProvider")
                .and_then(Value::as_str)
                .and_then(normalize_optional_text)
                .unwrap_or_else(|| "none".to_owned());
            let openai_key = status
                .get("hasOpenAIKey")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let elevenlabs_key = status
                .get("hasElevenLabsKey")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let kittentts = status
                .get("hasKittenTtsBinary")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            return Ok(format!(
                "TTS status: {}\nProvider: {}\nFallback: {}\nOpenAI key: {}\nElevenLabs key: {}\nKittenTTS binary: {}",
                if enabled { "enabled" } else { "disabled" },
                provider,
                fallback,
                if openai_key { "yes" } else { "no" },
                if elevenlabs_key { "yes" } else { "no" },
                if kittentts { "yes" } else { "no" }
            ));
        }
        if args[0].eq_ignore_ascii_case("providers") {
            let providers = self
                .gateway_rpc_call(
                    "tts.providers",
                    json!({}),
                    Duration::from_secs(15),
                    &["operator.read"],
                )
                .await?;
            let active = providers
                .get("active")
                .and_then(Value::as_str)
                .and_then(normalize_optional_text)
                .unwrap_or_else(|| "unknown".to_owned());
            let entries = providers
                .get("providers")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut out = format!("TTS providers (active: {active}):");
            for entry in entries.iter().take(12) {
                let id = entry
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(normalize_optional_text)
                    .unwrap_or_else(|| "<unknown>".to_owned());
                let configured = entry
                    .get("configured")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let _ = writeln!(
                    out,
                    "\n- {id} ({})",
                    if configured {
                        "configured"
                    } else {
                        "not configured"
                    }
                );
            }
            return Ok(out.trim_end().to_owned());
        }
        if args[0].eq_ignore_ascii_case("on") || args[0].eq_ignore_ascii_case("enable") {
            self.gateway_rpc_call(
                "tts.enable",
                json!({}),
                Duration::from_secs(15),
                &["operator.write"],
            )
            .await?;
            return Ok(
                "TTS enabled. New replies will include a Telegram audio clip when synthesis succeeds."
                    .to_owned(),
            );
        }
        if args[0].eq_ignore_ascii_case("off") || args[0].eq_ignore_ascii_case("disable") {
            self.gateway_rpc_call(
                "tts.disable",
                json!({}),
                Duration::from_secs(15),
                &["operator.write"],
            )
            .await?;
            return Ok("TTS disabled.".to_owned());
        }
        if args[0].eq_ignore_ascii_case("provider") {
            let Some(provider) = args.get(1).and_then(|value| normalize_optional_text(value))
            else {
                return Ok("Usage: /tts provider <openai|elevenlabs|kittentts|edge>".to_owned());
            };
            let result = self
                .gateway_rpc_call(
                    "tts.setProvider",
                    json!({
                        "provider": provider
                    }),
                    Duration::from_secs(15),
                    &["operator.write"],
                )
                .await?;
            let active = result
                .get("provider")
                .and_then(Value::as_str)
                .and_then(normalize_optional_text)
                .unwrap_or_else(|| "unknown".to_owned());
            return Ok(format!("TTS provider set: {active}"));
        }
        if args[0].eq_ignore_ascii_case("speak") {
            let text = args.iter().skip(1).copied().collect::<Vec<_>>().join(" ");
            let Some(input_text) = normalize_optional_text(&text) else {
                return Ok("Usage: /tts speak <text>".to_owned());
            };
            let payload = self
                .gateway_rpc_call(
                    "tts.convert",
                    json!({
                        "text": truncate_text(&input_text, TELEGRAM_TTS_INLINE_SAMPLE_MAX_CHARS),
                        "channel": "telegram",
                        "outputFormat": "wav",
                        "requireRealAudio": false
                    }),
                    Duration::from_secs(30),
                    &["operator.read", "operator.write"],
                )
                .await?;
            let clip = parse_tts_audio_clip(&payload)?;
            let provider_used = clip
                .provider_used
                .clone()
                .unwrap_or_else(|| "unknown".to_owned());
            let source = clip.source.clone().unwrap_or_else(|| "unknown".to_owned());
            self.send_audio_clip(token, chat_id, None, reply_to_message_id, clip)
                .await?;
            return Ok(format!(
                "TTS clip sent (providerUsed: {provider_used}, source: {source})."
            ));
        }
        Ok(format!(
            "Unknown /tts subcommand `{}`.\n\n{}",
            args[0],
            format_tts_help()
        ))
    }

    async fn handle_model_command(
        &self,
        session_key: &str,
        raw_args: &str,
    ) -> Result<String, String> {
        let catalog = self.fetch_model_catalog().await?;
        let args = raw_args
            .split_whitespace()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if args.is_empty() || args[0].eq_ignore_ascii_case("help") {
            return Ok(format_model_help(&catalog));
        }
        if args[0].eq_ignore_ascii_case("list") {
            let provider_filter = args
                .get(1)
                .map(|value| normalize_provider_alias(value))
                .and_then(|value| normalize_optional_text(&value));
            return Ok(format_model_list(&catalog, provider_filter.as_deref()));
        }

        let (provider, requested_model, matched_catalog_model) =
            resolve_model_selection(&catalog, &args)?;
        let patch_value = format!("{provider}/{requested_model}");
        self.patch_session_model_override(session_key, Some(&patch_value))
            .await?;
        let patch_result = self
            .gateway_rpc_call(
                "session.status",
                json!({
                    "sessionKey": session_key
                }),
                Duration::from_secs(15),
                &["operator.admin"],
            )
            .await?;
        let resolved_provider = patch_result
            .pointer("/resolved/modelProvider")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text)
            .unwrap_or(provider);
        let resolved_model = patch_result
            .pointer("/resolved/model")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text)
            .unwrap_or(requested_model);
        let mut response = format!("Model set: {resolved_provider}/{resolved_model}");
        if !matched_catalog_model {
            response.push_str("\nNote: custom model override was applied (not found in catalog).");
        }
        Ok(response)
    }

    async fn handle_set_api_key_command(&self, raw_args: &str) -> Result<String, String> {
        let args = raw_args
            .split_whitespace()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if args.len() < 4
            || !args[0].eq_ignore_ascii_case("api")
            || !args[1].eq_ignore_ascii_case("key")
        {
            return Ok(
                "Usage: /set api key <provider> <key>\nExample: /set api key openrouter sk-..."
                    .to_owned(),
            );
        }
        let provider = normalize_provider_alias(args[2]);
        let key = args[3..].join(" ");
        let Some(api_key) = normalize_optional_text(&key) else {
            return Ok(
                "Usage: /set api key <provider> <key>\nExample: /set api key groq gsk_..."
                    .to_owned(),
            );
        };
        if api_key.contains('\n') || api_key.contains('\r') {
            return Err("api key must be a single line".to_owned());
        }

        let config_result = self
            .gateway_rpc_call(
                "config.get",
                json!({}),
                Duration::from_secs(15),
                &["operator.read", "operator.write"],
            )
            .await?;
        let Some(base_hash) = config_result
            .get("hash")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text)
        else {
            return Err("config.get response missing hash".to_owned());
        };
        let patch = json!({
            "models": {
                "providers": {
                    provider: {
                        "apiKey": api_key
                    }
                }
            }
        });
        let raw_patch = serde_json::to_string(&patch)
            .map_err(|err| format!("failed serializing patch: {err}"))?;
        self.gateway_rpc_call(
            "config.patch",
            json!({
                "raw": raw_patch,
                "baseHash": base_hash
            }),
            Duration::from_secs(20),
            &["operator.admin"],
        )
        .await?;
        Ok(
            "Provider API key saved. You can now set a model with /model <provider>/<model>."
                .to_owned(),
        )
    }

    async fn fetch_model_catalog(&self) -> Result<Vec<CatalogModel>, String> {
        let result = self
            .gateway_rpc_call(
                "models.list",
                json!({}),
                Duration::from_secs(15),
                &["operator.read"],
            )
            .await?;
        let models = result
            .get("models")
            .and_then(Value::as_array)
            .ok_or_else(|| "models.list result missing models array".to_owned())?;
        let mut out = Vec::new();
        for entry in models {
            let Some(provider_raw) = entry.get("provider").and_then(Value::as_str) else {
                continue;
            };
            let Some(id_raw) = entry.get("id").and_then(Value::as_str) else {
                continue;
            };
            let provider = normalize_provider_alias(provider_raw);
            let Some(id) = normalize_optional_text(id_raw) else {
                continue;
            };
            let name = entry
                .get("name")
                .and_then(Value::as_str)
                .and_then(normalize_optional_text)
                .unwrap_or_else(|| id.clone());
            out.push(CatalogModel { provider, id, name });
        }
        out.sort_by(|left, right| {
            left.provider
                .cmp(&right.provider)
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.id.cmp(&right.id))
        });
        out.dedup_by(|left, right| {
            left.provider.eq_ignore_ascii_case(&right.provider)
                && left.id.eq_ignore_ascii_case(&right.id)
        });
        Ok(out)
    }

    async fn run_agent_with_fallback(
        &self,
        session_key: &str,
        text: &str,
        chat_id: i64,
        thread_id: Option<String>,
        settings: &TelegramSettings,
        update_id: u64,
    ) -> Result<String, String> {
        let mut attempted = HashSet::new();
        match self
            .run_agent_once(
                session_key,
                text,
                chat_id,
                thread_id.clone(),
                update_id,
                "base",
            )
            .await
        {
            Ok(reply) => return Ok(reply),
            Err(err) => {
                attempted.insert(("".to_owned(), "".to_owned()));
                debug!("telegram base attempt failed: {err}");
            }
        }

        let original_model_override = self.read_session_model_override(session_key).await;
        let mut last_error = "agent execution failed".to_owned();
        for candidate in &settings.candidates {
            let key = (candidate.provider.clone(), candidate.model.clone());
            if attempted.contains(&key) {
                continue;
            }
            attempted.insert(key);
            let patch_value = format!("{}/{}", candidate.provider, candidate.model);
            if let Err(err) = self
                .patch_session_model_override(session_key, Some(&patch_value))
                .await
            {
                last_error = format!(
                    "sessions.patch failed for {}/{}: {err}",
                    candidate.provider, candidate.model
                );
                continue;
            }

            match self
                .run_agent_once(
                    session_key,
                    text,
                    chat_id,
                    thread_id.clone(),
                    update_id,
                    &format!("{}-{}", candidate.provider, candidate.model),
                )
                .await
            {
                Ok(reply) => {
                    if let Err(err) = self
                        .patch_session_model_override(
                            session_key,
                            original_model_override.as_deref(),
                        )
                        .await
                    {
                        debug!(
                            "telegram fallback restore skipped after success for session {}: {}",
                            session_key, err
                        );
                    }
                    return Ok(reply);
                }
                Err(err) => {
                    last_error = err;
                }
            }
        }
        if let Err(err) = self
            .patch_session_model_override(session_key, original_model_override.as_deref())
            .await
        {
            debug!(
                "telegram fallback restore skipped after failure for session {}: {}",
                session_key, err
            );
        }
        Err(last_error)
    }

    async fn run_agent_once(
        &self,
        session_key: &str,
        text: &str,
        chat_id: i64,
        thread_id: Option<String>,
        update_id: u64,
        attempt_id: &str,
    ) -> Result<String, String> {
        let run_id = format!("telegram-{update_id}-{attempt_id}-{}", now_ms());
        let agent_request = json!({
            "idempotencyKey": run_id,
            "sessionKey": session_key,
            "message": text,
            "channel": "telegram",
            "to": chat_id.to_string(),
            "accountId": TELEGRAM_ACCOUNT_ID,
            "threadId": thread_id
        });
        let run_result = self
            .gateway_rpc_call(
                "agent",
                agent_request,
                Duration::from_millis(AGENT_RPC_TIMEOUT_MS),
                &["operator.admin"],
            )
            .await?;
        let resolved_run_id = run_result
            .get("runId")
            .and_then(Value::as_str)
            .and_then(normalize_optional_text)
            .unwrap_or_else(|| run_id.clone());

        let wait_result = self
            .gateway_rpc_call(
                "agent.wait",
                json!({
                    "runId": resolved_run_id,
                    "timeoutMs": AGENT_WAIT_TIMEOUT_MS
                }),
                Duration::from_secs((AGENT_WAIT_TIMEOUT_MS / 1_000).saturating_add(15)),
                &["operator.admin"],
            )
            .await?;
        let status = wait_result
            .get("status")
            .and_then(Value::as_str)
            .map(|raw| raw.to_ascii_lowercase())
            .unwrap_or_else(|| "unknown".to_owned());
        if status != "ok" {
            let reason = wait_result
                .get("error")
                .and_then(Value::as_str)
                .and_then(normalize_optional_text)
                .or_else(|| {
                    run_result
                        .pointer("/runtime/reason")
                        .and_then(Value::as_str)
                        .and_then(normalize_optional_text)
                })
                .unwrap_or_else(|| format!("agent run finished with status {status}"));
            return Err(reason);
        }

        let history = self
            .gateway_rpc_call(
                "sessions.history",
                json!({
                    "sessionKey": session_key,
                    "limit": 80
                }),
                Duration::from_secs(15),
                &["operator.admin"],
            )
            .await?;
        let reply = extract_assistant_reply(&history, &resolved_run_id)
            .ok_or_else(|| "agent completed without assistant text".to_owned())?;
        Ok(reply)
    }

    async fn gateway_rpc_call(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        scopes: &[&str],
    ) -> Result<Value, String> {
        let (mut ws, _) = connect_async(&self.gateway_ws_url)
            .await
            .map_err(|err| format!("gateway websocket connect failed: {err}"))?;

        let connect_id = format!("telegram-bridge-connect-{}", now_ms());
        let mut connect_payload = json!({
            "type": "req",
            "id": connect_id,
            "method": "connect",
            "params": {
                "role": "operator",
                "scopes": scopes,
                "client": {
                    "id": "openclaw-agent-rs.telegram-bridge"
                }
            }
        });
        if let Some(token) = self
            .gateway
            .token
            .clone()
            .as_deref()
            .and_then(normalize_optional_text)
        {
            connect_payload["params"]["auth"]["token"] = Value::String(token);
        }
        if let Some(password) = self
            .gateway
            .password
            .clone()
            .as_deref()
            .and_then(normalize_optional_text)
        {
            connect_payload["params"]["auth"]["password"] = Value::String(password);
        }
        ws.send(Message::Text(connect_payload.to_string()))
            .await
            .map_err(|err| format!("gateway connect request send failed: {err}"))?;
        let connect_frame = read_response_frame(&mut ws, &connect_id, timeout).await?;
        if !connect_frame
            .get("ok")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let message = connect_frame
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("gateway connect rejected");
            return Err(message.to_owned());
        }

        let request_id = format!("telegram-bridge-{method}-{}", now_ms());
        let request = json!({
            "type": "req",
            "id": request_id,
            "method": method,
            "params": params
        });
        ws.send(Message::Text(request.to_string()))
            .await
            .map_err(|err| format!("gateway request send failed ({method}): {err}"))?;
        let response = read_response_frame(&mut ws, &request_id, timeout).await?;
        if !response.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            let message = response
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("gateway request failed");
            return Err(message.to_owned());
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn emit_status_event(&self, settings: &TelegramSettings) -> Result<(), String> {
        self.gateway_emit_event(
            "telegram.status",
            json!({
                "channel": "telegram",
                "accountId": TELEGRAM_ACCOUNT_ID,
                "configured": true,
                "linked": true,
                "enabled": true,
                "running": true,
                "connected": true,
                "mode": "polling",
                "botTokenSource": "config",
                "dmPolicy": settings.dm_policy,
                "allowFrom": settings.allow_from
            }),
        )
        .await
    }

    async fn emit_inbound_event(&self, update_id: u64) -> Result<(), String> {
        self.gateway_emit_event(
            "telegram.message.received",
            json!({
                "channel": "telegram",
                "accountId": TELEGRAM_ACCOUNT_ID,
                "updateId": update_id
            }),
        )
        .await
    }

    async fn emit_outbound_event(&self, update_id: u64) -> Result<(), String> {
        self.gateway_emit_event(
            "telegram.message.sent",
            json!({
                "channel": "telegram",
                "accountId": TELEGRAM_ACCOUNT_ID,
                "updateId": update_id
            }),
        )
        .await
    }

    async fn gateway_emit_event(&self, event: &str, payload: Value) -> Result<(), String> {
        let (mut ws, _) = connect_async(&self.gateway_ws_url)
            .await
            .map_err(|err| format!("gateway websocket connect failed: {err}"))?;
        let connect_id = format!("telegram-bridge-event-connect-{}", now_ms());
        let connect_request = json!({
            "type": "req",
            "id": connect_id,
            "method": "connect",
            "params": {
                "role": "operator",
                "scopes": ["operator.admin"],
                "client": {
                    "id": "openclaw-agent-rs.telegram-bridge"
                }
            }
        });
        ws.send(Message::Text(connect_request.to_string()))
            .await
            .map_err(|err| format!("gateway event connect send failed: {err}"))?;
        let connect_frame =
            read_response_frame(&mut ws, &connect_id, Duration::from_secs(10)).await?;
        if !connect_frame
            .get("ok")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let message = connect_frame
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("event connect rejected");
            return Err(message.to_owned());
        }
        let frame = json!({
            "type": "event",
            "event": event,
            "payload": payload
        });
        ws.send(Message::Text(frame.to_string()))
            .await
            .map_err(|err| format!("event send failed: {err}"))?;
        Ok(())
    }

    async fn save_offset(&self, offset: u64) -> Result<(), String> {
        if let Some(parent) = self.offset_path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|err| format!("failed creating offset dir {}: {err}", parent.display()))?;
        }
        let temp_path = self.offset_path.with_extension("tmp");
        let payload = json!({ "offset": offset });
        fs::write(
            &temp_path,
            serde_json::to_vec(&payload)
                .map_err(|err| format!("failed serializing offset payload: {err}"))?,
        )
        .await
        .map_err(|err| {
            format!(
                "failed writing offset temp file {}: {err}",
                temp_path.display()
            )
        })?;
        fs::rename(&temp_path, &self.offset_path)
            .await
            .map_err(|err| {
                format!(
                    "failed replacing offset file {}: {err}",
                    self.offset_path.display()
                )
            })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        allows_group_message, build_session_key, derive_gateway_ws_url, extract_assistant_reply,
        extract_model_candidates, extract_telegram_settings, is_allowed_by_dm_policy,
        normalize_provider_alias, parse_auth_wait_timeout_ms, parse_telegram_control_command,
        resolve_model_selection, to_bridge_health_url, CatalogModel, TelegramControlCommand,
        AGENT_RPC_TIMEOUT_MS, AGENT_WAIT_TIMEOUT_MS,
    };
    use crate::config::Config;
    use serde_json::json;

    #[test]
    fn derive_gateway_ws_url_prefers_bind_port() {
        let mut cfg = Config::default();
        cfg.gateway.url = "ws://10.0.0.5:9999/ws".to_owned();
        cfg.gateway.server.bind = "0.0.0.0:18789".to_owned();
        assert_eq!(
            derive_gateway_ws_url(&cfg.gateway),
            "ws://127.0.0.1:18789/ws"
        );
    }

    #[test]
    fn extract_telegram_settings_reads_token_and_allowlist() {
        let config = json!({
            "channels": {
                "telegram": {
                    "botToken": "123:abc",
                    "dmPolicy": "allowlist",
                    "groupPolicy": "mention",
                    "allowFrom": ["telegram:42", "@Alice", "  "]
                }
            },
            "models": {
                "providers": {
                    "opencodefree": {
                        "models": [{"id": "kimi-k2.5-free"}, {"id": "minimax-m2.5-free"}]
                    },
                    "zhipu": {
                        "models": [{"id": "glm-5"}]
                    }
                }
            }
        });
        let settings = extract_telegram_settings(&config).expect("telegram settings");
        assert_eq!(settings.bot_token, "123:abc");
        assert_eq!(settings.dm_policy, "allowlist");
        assert_eq!(settings.group_policy, "mention");
        assert!(settings.allow_from.contains(&"telegram:42".to_owned()));
        assert!(settings.allow_from.contains(&"@alice".to_owned()));
        assert_eq!(settings.candidates.len(), 3);
        assert_eq!(settings.candidates[0].provider, "opencode");
    }

    #[test]
    fn allowlist_matches_numeric_and_username_tags() {
        let message = json!({
            "from": {"id": 7_670_750_155_i64, "username": "AdyUser"},
            "chat": {"id": 7_670_750_155_i64}
        });
        assert!(is_allowed_by_dm_policy(
            &message,
            "allowlist",
            &["telegram:7670750155".to_owned()]
        ));
        assert!(is_allowed_by_dm_policy(
            &message,
            "allowlist",
            &["@adyuser".to_owned()]
        ));
        assert!(!is_allowed_by_dm_policy(
            &message,
            "allowlist",
            &["@someoneelse".to_owned()]
        ));
    }

    #[test]
    fn group_policy_requires_mention_when_enabled() {
        let message = json!({
            "text": "hello @OpenClawBot please respond",
            "chat": {"type": "group"}
        });
        assert!(allows_group_message(
            &message,
            "group",
            "mention",
            11,
            Some("openclawbot")
        ));
        let plain = json!({
            "text": "hello everyone",
            "chat": {"type": "group"}
        });
        assert!(!allows_group_message(
            &plain,
            "group",
            "mention",
            11,
            Some("openclawbot")
        ));
        assert!(allows_group_message(
            &plain,
            "group",
            "all",
            11,
            Some("openclawbot")
        ));
    }

    #[test]
    fn session_key_uses_dm_and_topic_variants() {
        let dm = json!({
            "chat": {"id": 99, "type": "private"},
            "from": {"id": 42}
        });
        assert_eq!(build_session_key(&dm), "agent:main:telegram:dm:42");
        let topic = json!({
            "chat": {"id": -100123, "type": "supergroup"},
            "from": {"id": 42},
            "message_thread_id": 77
        });
        assert_eq!(
            build_session_key(&topic),
            "agent:main:telegram:group:-100123:topic:77"
        );
    }

    #[test]
    fn extract_assistant_reply_prefers_matching_run_id() {
        let history = json!({
            "history": [
                {"source": "agent.assistant", "requestId": "run-a", "text": "older"},
                {"source": "agent.assistant", "requestId": "run-b", "text": "target"}
            ]
        });
        assert_eq!(
            extract_assistant_reply(&history, "run-b").as_deref(),
            Some("target")
        );
    }

    #[test]
    fn model_candidates_normalize_provider_aliases() {
        let config = json!({
            "models": {
                "providers": {
                    "z.ai": {
                        "models": [{"id":"glm-5"}]
                    },
                    "opencode_free": {
                        "models": [{"id":"kimi-k2.5-free"}]
                    },
                    "qwen": {
                        "models": [{"id":"qwen3.5-397b-a17b"}]
                    },
                    "mercury": {
                        "models": [{"id":"mercury-2"}]
                    }
                }
            }
        });
        let candidates = extract_model_candidates(&config);
        assert_eq!(candidates.len(), 4);
        assert_eq!(candidates[0].provider, "opencode");
        assert_eq!(candidates[1].provider, "qwen-portal");
        assert_eq!(candidates[2].provider, "zai");
        assert_eq!(candidates[3].provider, "inception");
    }

    #[test]
    fn parse_telegram_control_commands_support_model_set_api_key_auth_and_tts() {
        let model =
            parse_telegram_control_command("/model@OpenClawBot list qwen").expect("model command");
        match model {
            TelegramControlCommand::Model { raw_args } => assert_eq!(raw_args, "list qwen"),
            _ => panic!("expected model command"),
        }

        let set =
            parse_telegram_control_command("/set api key openrouter sk-123").expect("set command");
        match set {
            TelegramControlCommand::SetApiKey { raw_args } => {
                assert_eq!(raw_args, "api key openrouter sk-123")
            }
            _ => panic!("expected set api key command"),
        }

        let auth =
            parse_telegram_control_command("/auth start kimi --force").expect("auth command");
        match auth {
            TelegramControlCommand::Auth { raw_args } => assert_eq!(raw_args, "start kimi --force"),
            _ => panic!("expected auth command"),
        }

        let tts = parse_telegram_control_command("/tts speak hello world").expect("tts command");
        match tts {
            TelegramControlCommand::Tts { raw_args } => assert_eq!(raw_args, "speak hello world"),
            _ => panic!("expected tts command"),
        }
    }

    #[test]
    fn normalize_provider_alias_maps_models_dev_variants() {
        assert_eq!(normalize_provider_alias("fireworks-ai"), "fireworks");
        assert_eq!(normalize_provider_alias("moonshotai"), "moonshot");
        assert_eq!(normalize_provider_alias("novita-ai"), "novita");
        assert_eq!(normalize_provider_alias("opencode-go"), "opencode");
        assert_eq!(normalize_provider_alias("kimi-for-coding"), "kimi-coding");
        assert_eq!(normalize_provider_alias("zaiweb"), "zai");
    }

    #[test]
    fn auth_wait_timeout_parser_supports_seconds_and_ms_flags() {
        assert_eq!(
            parse_auth_wait_timeout_ms(&["wait", "openai", "--timeout", "90"]),
            90_000
        );
        assert_eq!(
            parse_auth_wait_timeout_ms(&["wait", "openai", "--timeout-ms", "120000"]),
            120_000
        );
        assert_eq!(
            parse_auth_wait_timeout_ms(&["wait", "openai", "--timeout=45"]),
            45_000
        );
        assert_eq!(
            parse_auth_wait_timeout_ms(&["wait", "openai", "--timeout-ms=2500"]),
            5_000
        );
    }

    #[test]
    fn bridge_health_url_normalizes_v1_candidates() {
        assert_eq!(
            to_bridge_health_url("http://127.0.0.1:43110/v1").as_deref(),
            Some("http://127.0.0.1:43110/health")
        );
        assert_eq!(
            to_bridge_health_url("http://127.0.0.1:43110/v1/chat/completions").as_deref(),
            Some("http://127.0.0.1:43110/health")
        );
    }

    #[test]
    fn resolve_model_selection_defaults_to_provider_first_catalog_entry() {
        let catalog = vec![
            CatalogModel {
                provider: "openrouter".to_owned(),
                id: "qwen/qwen3-coder:free".to_owned(),
                name: "Qwen3 Coder".to_owned(),
            },
            CatalogModel {
                provider: "openrouter".to_owned(),
                id: "inception/mercury".to_owned(),
                name: "Inception Mercury".to_owned(),
            },
        ];
        let (provider, model, from_catalog) =
            resolve_model_selection(&catalog, &["openrouter"]).expect("resolve");
        assert_eq!(provider, "openrouter");
        assert_eq!(model, "qwen/qwen3-coder:free");
        assert!(from_catalog);
    }

    #[test]
    fn agent_rpc_timeout_budget_exceeds_agent_wait_timeout() {
        let rpc_timeout = std::hint::black_box(AGENT_RPC_TIMEOUT_MS);
        let wait_timeout = std::hint::black_box(AGENT_WAIT_TIMEOUT_MS);
        assert!(rpc_timeout > wait_timeout);
    }
}
