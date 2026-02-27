use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;
use url::form_urlencoded;

#[derive(Debug, Clone)]
pub struct WebsiteBridgeRequest<'a> {
    pub provider: &'a str,
    pub model: &'a str,
    pub messages: &'a [Value],
    pub tools: &'a [Value],
    pub timeout_ms: u64,
    pub website_url: Option<&'a str>,
    pub candidate_base_urls: &'a [String],
    pub headers: &'a [(String, String)],
    pub auth_header_name: &'a str,
    pub auth_header_prefix: &'a str,
    pub api_key: Option<&'a str>,
    pub request_overrides: &'a serde_json::Map<String, Value>,
}

#[derive(Debug, Clone)]
pub struct WebsiteBridgeResponse {
    pub body: String,
    pub endpoint: String,
}

const ZAI_SIGNING_KEY: &str = "key-@@@@)))()((9))-xxxx&&&%%%%%";
const ZAI_FE_VERSION: &str = "prod-fe-1.0.241";
static ZAI_COUNTER: AtomicU64 = AtomicU64::new(1);
static CHATGPT_COUNTER: AtomicU64 = AtomicU64::new(1);

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Deserialize)]
struct ZaiGuestAuthResponse {
    id: String,
    token: String,
    #[serde(default)]
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ZaiChatCreateResponse {
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct QwenGuestAuthResponse {
    token: String,
}

#[derive(Debug, Clone, Deserialize)]
struct QwenChatCreateResponse {
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct InceptionGuestAuthResponse {
    token: String,
}

#[derive(Debug, Clone, Deserialize)]
struct InceptionChatCreateResponse {
    id: String,
}

pub async fn invoke_openai_compatible(
    request: WebsiteBridgeRequest<'_>,
) -> Result<WebsiteBridgeResponse, String> {
    let timeout = Duration::from_millis(request.timeout_ms.max(1_000));
    let client = Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|err| format!("failed creating website bridge client: {err}"))?;

    let website_online = probe_website_status(&client, request.website_url).await;

    let mut attempts = Vec::new();
    let mut candidate_endpoints = request
        .candidate_base_urls
        .iter()
        .filter_map(|raw| normalize_optional_text(raw, 2_048))
        .map(|base| resolve_chat_completion_endpoint(&base))
        .collect::<Vec<_>>();
    candidate_endpoints.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    if candidate_endpoints.is_empty() && attempts.is_empty() {
        attempts.push("website bridge has no candidate endpoints configured".to_owned());
    }
    if should_use_chatgpt_web_bridge(&request) {
        match invoke_chatgpt_web_bridge(&client, &request).await {
            Ok(response) => return Ok(response),
            Err(err) => attempts.push(format!("official_chatgpt_bridge_error: {err}")),
        }
    }

    // Prefer local loopback bridges first when present. This gives deterministic
    // E2E behavior on hosts that run a local browser-assisted bridge.
    let loopback_endpoints = candidate_endpoints
        .iter()
        .filter(|endpoint| endpoint_targets_loopback(endpoint))
        .cloned()
        .collect::<Vec<_>>();
    let prioritize_loopback = !loopback_endpoints.is_empty()
        && (matches_zai_guest_bridge(&request)
            || matches_qwen_guest_bridge(&request)
            || matches_inception_guest_bridge(&request));
    if prioritize_loopback {
        if let Some(response) =
            try_candidate_endpoints(&client, &request, &loopback_endpoints, &mut attempts).await
        {
            return Ok(response);
        }
    }

    let mut zai_guest_attempted = false;
    let mut qwen_guest_attempted = false;
    let mut inception_guest_attempted = false;
    if should_use_zai_guest_bridge(&request) {
        zai_guest_attempted = true;
        match invoke_zai_guest_bridge(&client, &request).await {
            Ok(response) => return Ok(response),
            Err(err) => attempts.push(format!("official_zai_bridge_error: {err}")),
        }
    }
    if should_use_qwen_guest_bridge(&request) {
        qwen_guest_attempted = true;
        match invoke_qwen_guest_bridge(&client, &request).await {
            Ok(response) => return Ok(response),
            Err(err) => attempts.push(format!("official_qwen_bridge_error: {err}")),
        }
    }
    if should_use_inception_guest_bridge(&request) {
        inception_guest_attempted = true;
        match invoke_inception_guest_bridge(&client, &request).await {
            Ok(response) => return Ok(response),
            Err(err) => attempts.push(format!("official_inception_bridge_error: {err}")),
        }
    }

    let remaining_endpoints = if prioritize_loopback {
        candidate_endpoints
            .iter()
            .filter(|endpoint| !endpoint_targets_loopback(endpoint))
            .cloned()
            .collect::<Vec<_>>()
    } else {
        candidate_endpoints.clone()
    };
    if let Some(response) =
        try_candidate_endpoints(&client, &request, &remaining_endpoints, &mut attempts).await
    {
        return Ok(response);
    }

    if !zai_guest_attempted && matches_zai_guest_bridge(&request) {
        match invoke_zai_guest_bridge(&client, &request).await {
            Ok(response) => return Ok(response),
            Err(err) => attempts.push(format!("official_zai_bridge_fallback_error: {err}")),
        }
    }
    if !qwen_guest_attempted && matches_qwen_guest_bridge(&request) {
        match invoke_qwen_guest_bridge(&client, &request).await {
            Ok(response) => return Ok(response),
            Err(err) => attempts.push(format!("official_qwen_bridge_fallback_error: {err}")),
        }
    }
    if !inception_guest_attempted && matches_inception_guest_bridge(&request) {
        match invoke_inception_guest_bridge(&client, &request).await {
            Ok(response) => return Ok(response),
            Err(err) => attempts.push(format!("official_inception_bridge_fallback_error: {err}")),
        }
    }

    let online_hint = match website_online {
        Some(true) => "website=online",
        Some(false) => "website=offline",
        None => "website=unknown",
    };
    Err(format!(
        "website bridge failed for provider {} ({online_hint}); attempts: {}",
        request.provider,
        attempts.join(" | ")
    ))
}

async fn try_candidate_endpoints(
    client: &Client,
    request: &WebsiteBridgeRequest<'_>,
    candidate_endpoints: &[String],
    attempts: &mut Vec<String>,
) -> Option<WebsiteBridgeResponse> {
    for endpoint in candidate_endpoints {
        let mut request_builder = client
            .post(endpoint)
            .header("Content-Type", "application/json");
        for (name, value) in request.headers {
            request_builder = request_builder.header(name, value);
        }
        if let Some(api_key) = request
            .api_key
            .and_then(|value| normalize_optional_text(value, 4096))
        {
            request_builder = request_builder.header(
                request.auth_header_name,
                format!("{}{}", request.auth_header_prefix, api_key),
            );
        }

        let mut payload = request.request_overrides.clone();
        payload.insert("model".to_owned(), Value::String(request.model.to_owned()));
        payload.insert(
            "messages".to_owned(),
            Value::Array(request.messages.to_vec()),
        );
        payload
            .entry("stream".to_owned())
            .or_insert(Value::Bool(false));
        if !request.tools.is_empty() {
            payload.insert("tools".to_owned(), Value::Array(request.tools.to_vec()));
            payload
                .entry("tool_choice".to_owned())
                .or_insert(Value::String("auto".to_owned()));
        }

        let response = match request_builder.json(&Value::Object(payload)).send().await {
            Ok(value) => value,
            Err(err) => {
                attempts.push(format!("{endpoint}: transport_error: {err}"));
                continue;
            }
        };
        let status = resolve_effective_status(&response);
        let body = match response.text().await {
            Ok(value) => value,
            Err(err) => {
                attempts.push(format!("{endpoint}: body_read_error: {err}"));
                continue;
            }
        };
        if !status.is_success() {
            attempts.push(format!(
                "{endpoint}: status={} body={}",
                status.as_u16(),
                truncate_text(&body, 240)
            ));
            continue;
        }
        let parsed: Value = match serde_json::from_str(&body) {
            Ok(value) => value,
            Err(err) => {
                attempts.push(format!("{endpoint}: parse_error: {err}"));
                continue;
            }
        };
        if openai_response_has_usable_assistant_output(&parsed) {
            return Some(WebsiteBridgeResponse {
                body,
                endpoint: endpoint.to_owned(),
            });
        }
        attempts.push(format!(
            "{endpoint}: missing_coherent_reply body={}",
            truncate_text(&parsed.to_string(), 240)
        ));
    }
    None
}

async fn probe_website_status(client: &Client, website_url: Option<&str>) -> Option<bool> {
    let website_url = website_url.and_then(|raw| normalize_optional_text(raw, 2_048))?;
    let response = client.get(website_url).send().await.ok()?;
    Some(response.status().is_success())
}

fn resolve_chat_completion_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return "/v1/chat/completions".to_owned();
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("/chat/completions") {
        return trimmed.to_owned();
    }
    if lower.ends_with("/v1") {
        return format!("{trimmed}/chat/completions");
    }
    format!("{trimmed}/v1/chat/completions")
}

fn resolve_effective_status(response: &reqwest::Response) -> StatusCode {
    let Some(raw) = response
        .headers()
        .get("x-actual-status-code")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u16>().ok())
    else {
        return response.status();
    };
    StatusCode::from_u16(raw).unwrap_or_else(|_| response.status())
}

fn endpoint_targets_loopback(endpoint: &str) -> bool {
    let Ok(url) = Url::parse(endpoint) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let normalized_host = host.trim_matches(['[', ']']);
    normalized_host.eq_ignore_ascii_case("localhost")
        || normalized_host == "127.0.0.1"
        || normalized_host == "::1"
}

fn normalize_optional_text(value: &str, max_len: usize) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len() <= max_len {
        return Some(trimmed.to_owned());
    }
    let mut end = max_len;
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = trimmed[..end].to_owned();
    out.push_str("...");
    Some(out)
}

fn truncate_text(value: &str, max_len: usize) -> String {
    if value.len() <= max_len {
        return value.to_owned();
    }
    let mut end = max_len;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = value[..end].to_owned();
    out.push_str("...");
    out
}

fn should_use_zai_guest_bridge(request: &WebsiteBridgeRequest<'_>) -> bool {
    !has_api_key(request) && matches_zai_guest_bridge(request)
}

fn should_use_qwen_guest_bridge(request: &WebsiteBridgeRequest<'_>) -> bool {
    matches_qwen_guest_bridge(request)
}

fn should_use_inception_guest_bridge(request: &WebsiteBridgeRequest<'_>) -> bool {
    !has_api_key(request) && matches_inception_guest_bridge(request)
}

fn should_use_chatgpt_web_bridge(request: &WebsiteBridgeRequest<'_>) -> bool {
    has_api_key(request) && matches_chatgpt_web_bridge(request)
}

fn has_api_key(request: &WebsiteBridgeRequest<'_>) -> bool {
    request
        .api_key
        .and_then(|value| normalize_optional_text(value, 4_096))
        .is_some()
}

fn matches_chatgpt_web_bridge(request: &WebsiteBridgeRequest<'_>) -> bool {
    let provider = request.provider.trim().to_ascii_lowercase();
    if provider != "openai" && !provider.contains("chatgpt") {
        return false;
    }
    request.website_url.is_some_and(chatgpt_url_hint_matches)
        || request
            .candidate_base_urls
            .iter()
            .any(|url| chatgpt_url_hint_matches(url))
}

fn chatgpt_url_hint_matches(raw: &str) -> bool {
    let lowered = raw.trim().to_ascii_lowercase();
    lowered.contains("chatgpt.com") || lowered.contains("chat.openai.com")
}

fn matches_zai_guest_bridge(request: &WebsiteBridgeRequest<'_>) -> bool {
    let provider = request.provider.trim().to_ascii_lowercase();
    if provider.contains("zai") || provider.contains("zhipu") {
        return true;
    }
    request
        .website_url
        .map(|url| url.to_ascii_lowercase().contains("chat.z.ai"))
        .unwrap_or(false)
        || request
            .candidate_base_urls
            .iter()
            .any(|url| url.to_ascii_lowercase().contains("chat.z.ai"))
}

fn matches_qwen_guest_bridge(request: &WebsiteBridgeRequest<'_>) -> bool {
    let provider = request.provider.trim().to_ascii_lowercase();
    if provider.contains("qwen") {
        return true;
    }
    request
        .website_url
        .map(|url| {
            let lowered = url.to_ascii_lowercase();
            lowered.contains("chat.qwen.ai") || lowered.contains("qwen.ai")
        })
        .unwrap_or(false)
        || request.candidate_base_urls.iter().any(|url| {
            let lowered = url.to_ascii_lowercase();
            lowered.contains("chat.qwen.ai") || lowered.contains("qwen.ai")
        })
}

fn matches_inception_guest_bridge(request: &WebsiteBridgeRequest<'_>) -> bool {
    let provider = request.provider.trim().to_ascii_lowercase();
    if provider.contains("inception") || provider.contains("mercury") {
        return true;
    }
    request
        .website_url
        .map(|url| {
            let lowered = url.to_ascii_lowercase();
            lowered.contains("chat.inceptionlabs.ai") || lowered.contains("mercury")
        })
        .unwrap_or(false)
        || request.candidate_base_urls.iter().any(|url| {
            let lowered = url.to_ascii_lowercase();
            lowered.contains("chat.inceptionlabs.ai") || lowered.contains("mercury")
        })
}

fn openai_response_has_usable_assistant_output(parsed: &Value) -> bool {
    let Some(choices) = parsed.pointer("/choices").and_then(Value::as_array) else {
        return false;
    };
    if choices.is_empty() {
        return false;
    }
    choices.iter().any(|choice| {
        if extract_message_content_text(choice.pointer("/message/content")).is_some() {
            return true;
        }
        if choice
            .pointer("/delta/content")
            .and_then(Value::as_str)
            .and_then(|value| normalize_optional_text(value, 12_000))
            .is_some()
        {
            return true;
        }
        if choice
            .pointer("/message/tool_calls")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty())
        {
            return true;
        }
        choice
            .pointer("/tool_calls")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty())
    })
}

async fn invoke_zai_guest_bridge(
    client: &Client,
    request: &WebsiteBridgeRequest<'_>,
) -> Result<WebsiteBridgeResponse, String> {
    let origin = resolve_zai_origin(request)
        .ok_or_else(|| "zai guest bridge origin could not be resolved".to_owned())?;

    let auth_endpoint = format!("{origin}/api/v1/auths/");
    let auth_response = client
        .get(&auth_endpoint)
        .send()
        .await
        .map_err(|err| format!("zai auth request failed: {err}"))?;
    let auth_status = resolve_effective_status(&auth_response);
    let auth_body = auth_response
        .text()
        .await
        .map_err(|err| format!("zai auth body read failed: {err}"))?;
    if !auth_status.is_success() {
        return Err(format!(
            "zai auth request failed with status {}: {}",
            auth_status.as_u16(),
            truncate_text(&auth_body, 320)
        ));
    }
    let auth: ZaiGuestAuthResponse =
        serde_json::from_str(&auth_body).map_err(|err| format!("zai auth parse failed: {err}"))?;
    if auth.token.trim().is_empty() {
        return Err("zai auth response missing token".to_owned());
    }
    if auth.id.trim().is_empty() {
        return Err("zai auth response missing user id".to_owned());
    }

    let signature_prompt = extract_signature_prompt(request.messages)
        .ok_or_else(|| "zai guest bridge could not extract user prompt".to_owned())?;
    let model_candidates = build_zai_model_candidates(request.model);
    if model_candidates.is_empty() {
        return Err("zai guest bridge has no candidate models".to_owned());
    }
    let mut attempts = Vec::new();
    for model in model_candidates {
        match invoke_zai_guest_model(client, request, &origin, &auth, &signature_prompt, &model)
            .await
        {
            Ok(response) => return Ok(response),
            Err(err) => attempts.push(format!("{model}: {err}")),
        }
    }
    Err(format!(
        "zai guest bridge exhausted model candidates; attempts: {}",
        attempts.join(" | ")
    ))
}

async fn invoke_zai_guest_model(
    client: &Client,
    request: &WebsiteBridgeRequest<'_>,
    origin: &str,
    auth: &ZaiGuestAuthResponse,
    signature_prompt: &str,
    model: &str,
) -> Result<WebsiteBridgeResponse, String> {
    let user_message_id = zai_next_id("msg-user");
    let assistant_message_id = zai_next_id("msg-assistant");
    let create_chat_endpoint = format!("{origin}/api/v1/chats/new");
    let chat_payload = json!({
        "chat": {
            "title": "OpenClaw Rust",
            "models": [model],
            "params": request
                .request_overrides
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}))
        },
        "messages": [
            {
                "id": user_message_id,
                "role": "user",
                "content": signature_prompt
            }
        ]
    });
    let create_chat_response = client
        .post(&create_chat_endpoint)
        .header("Authorization", format!("Bearer {}", auth.token))
        .header("Content-Type", "application/json")
        .json(&chat_payload)
        .send()
        .await
        .map_err(|err| format!("zai chat create request failed: {err}"))?;
    let create_chat_status = resolve_effective_status(&create_chat_response);
    let create_chat_body = create_chat_response
        .text()
        .await
        .map_err(|err| format!("zai chat create body read failed: {err}"))?;
    if !create_chat_status.is_success() {
        return Err(format!(
            "zai chat create failed with status {}: {}",
            create_chat_status.as_u16(),
            truncate_text(&create_chat_body, 320)
        ));
    }
    let chat: ZaiChatCreateResponse = serde_json::from_str(&create_chat_body)
        .map_err(|err| format!("zai chat create parse failed: {err}"))?;
    if chat.id.trim().is_empty() {
        return Err("zai chat create response missing chat id".to_owned());
    }
    let timestamp_ms = zai_now_ms().to_string();
    let request_id = zai_next_id("request");
    let signature = zai_compute_signature(&request_id, &auth.id, signature_prompt, &timestamp_ms);
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("timestamp", &timestamp_ms)
        .append_pair("requestId", &request_id)
        .append_pair("user_id", &auth.id)
        .append_pair("version", "0.0.1")
        .append_pair("platform", "web")
        .append_pair("token", &auth.token)
        .append_pair("signature_timestamp", &timestamp_ms)
        .finish();
    let completion_endpoint = format!("{origin}/api/v2/chat/completions?{query}");

    let completion_payload = json!({
        "stream": false,
        "model": model,
        "messages": request.messages,
        "signature_prompt": signature_prompt,
        "params": request
            .request_overrides
            .get("params")
            .cloned()
            .unwrap_or_else(|| json!({})),
        "extra": request
            .request_overrides
            .get("extra")
            .cloned()
            .unwrap_or_else(|| json!({})),
        "features": request
            .request_overrides
            .get("features")
            .cloned()
            .unwrap_or_else(|| {
                json!({
                    "image_generation": false,
                    "web_search": false,
                    "auto_web_search": false,
                    "preview_mode": true,
                    "flags": [],
                    "enable_thinking": true
                })
            }),
        "variables": {
            "{{USER_NAME}}": normalize_optional_text(&auth.name, 128).unwrap_or_else(|| "Guest".to_owned()),
            "{{USER_LOCATION}}": "Unknown",
            "{{CURRENT_DATETIME}}": "1970-01-01 00:00:00",
            "{{CURRENT_DATE}}": "1970-01-01",
            "{{CURRENT_TIME}}": "00:00:00",
            "{{CURRENT_WEEKDAY}}": "Thursday",
            "{{CURRENT_TIMEZONE}}": "UTC",
            "{{USER_LANGUAGE}}": "en-US"
        },
        "chat_id": chat.id,
        "id": assistant_message_id,
        "current_user_message_id": user_message_id,
        "current_user_message_parent_id": Value::Null,
        "background_tasks": request
            .request_overrides
            .get("background_tasks")
            .cloned()
            .unwrap_or_else(|| {
                json!({
                    "title_generation": true,
                    "tags_generation": true
                })
            })
    });

    let mut completion_request = client
        .post(&completion_endpoint)
        .header("Authorization", format!("Bearer {}", auth.token))
        .header("Content-Type", "application/json")
        .header("Accept-Language", "en-US")
        .header("X-FE-Version", ZAI_FE_VERSION)
        .header("X-Signature", signature);
    for (name, value) in request.headers {
        completion_request = completion_request.header(name, value);
    }
    let completion_response = completion_request
        .json(&completion_payload)
        .send()
        .await
        .map_err(|err| format!("zai completion request failed: {err}"))?;
    let completion_status = resolve_effective_status(&completion_response);
    let completion_body = completion_response
        .text()
        .await
        .map_err(|err| format!("zai completion body read failed: {err}"))?;
    if !completion_status.is_success() {
        return Err(format!(
            "zai completion request failed with status {}: {}",
            completion_status.as_u16(),
            truncate_text(&completion_body, 640)
        ));
    }
    let openai_body = parse_zai_sse_to_openai_body(&completion_body).ok_or_else(|| {
        format!(
            "zai completion stream did not yield assistant content: {}",
            truncate_text(&completion_body, 640)
        )
    })?;
    Ok(WebsiteBridgeResponse {
        body: openai_body,
        endpoint: completion_endpoint,
    })
}

fn resolve_zai_origin(request: &WebsiteBridgeRequest<'_>) -> Option<String> {
    request
        .website_url
        .and_then(normalize_origin_url)
        .or_else(|| {
            request
                .candidate_base_urls
                .iter()
                .find_map(|candidate| normalize_origin_url(candidate))
        })
}

fn build_zai_model_candidates(model: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(requested) = normalize_optional_text(model, 256) {
        out.push(requested.clone());
        if let Some((_, model_id)) = requested.split_once('/') {
            if let Some(model_id) = normalize_optional_text(model_id, 256) {
                out.push(model_id);
            }
        }
        let key = normalize_model_alias_key(&requested);
        if key.contains("glm5") {
            out.push("glm-5".to_owned());
            out.push("glm-5-air".to_owned());
            out.push("glm-4.5".to_owned());
            out.push("glm-4.5-air".to_owned());
        } else if key.contains("glm45") {
            out.push("glm-4.5".to_owned());
            out.push("glm-4.5-air".to_owned());
            out.push("glm-5".to_owned());
        }
    }
    if out.is_empty() {
        out.push("glm-5".to_owned());
        out.push("glm-5-air".to_owned());
        out.push("glm-4.5".to_owned());
        out.push("glm-4.5-air".to_owned());
    }
    let mut dedup = Vec::with_capacity(out.len());
    for candidate in out {
        if dedup
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(&candidate))
        {
            continue;
        }
        dedup.push(candidate);
    }
    dedup
}

async fn invoke_inception_guest_bridge(
    client: &Client,
    request: &WebsiteBridgeRequest<'_>,
) -> Result<WebsiteBridgeResponse, String> {
    let origin = resolve_inception_origin(request)
        .ok_or_else(|| "inception bridge origin could not be resolved".to_owned())?;
    let model_candidates = build_inception_model_candidates(request.model);
    if model_candidates.is_empty() {
        return Err("inception bridge has no candidate models".to_owned());
    }
    let mut attempts = Vec::new();
    for model in &model_candidates {
        match invoke_inception_guest_model_direct(client, request, &origin, model).await {
            Ok(response) => return Ok(response),
            Err(err) => attempts.push(format!("{model}: direct {err}")),
        }
    }
    match resolve_inception_auth_token(client, request, &origin).await {
        Ok(Some(auth_token)) => {
            for model in &model_candidates {
                match invoke_inception_guest_model_legacy(
                    client,
                    request,
                    &origin,
                    &auth_token,
                    model,
                )
                .await
                {
                    Ok(response) => return Ok(response),
                    Err(err) => attempts.push(format!("{model}: legacy {err}")),
                }
            }
        }
        Ok(None) => attempts.push("legacy auth token missing".to_owned()),
        Err(err) => attempts.push(format!("legacy auth {err}")),
    }
    Err(format!(
        "inception bridge exhausted model candidates; attempts: {}",
        attempts.join(" | ")
    ))
}

fn resolve_inception_origin(request: &WebsiteBridgeRequest<'_>) -> Option<String> {
    request
        .website_url
        .and_then(normalize_origin_url)
        .or_else(|| {
            request
                .candidate_base_urls
                .iter()
                .find_map(|candidate| normalize_origin_url(candidate))
        })
        .or_else(|| Some("https://chat.inceptionlabs.ai".to_owned()))
}

async fn resolve_inception_auth_token(
    client: &Client,
    request: &WebsiteBridgeRequest<'_>,
    origin: &str,
) -> Result<Option<String>, String> {
    if let Some(token) = request
        .api_key
        .and_then(|value| normalize_optional_text(value, 8_192))
    {
        return Ok(Some(token));
    }
    let auth_endpoint = format!("{origin}/api/v1/auths/");
    let auth_response = client
        .get(&auth_endpoint)
        .send()
        .await
        .map_err(|err| format!("inception auth request failed: {err}"))?;
    let status = resolve_effective_status(&auth_response);
    let body = auth_response
        .text()
        .await
        .map_err(|err| format!("inception auth body read failed: {err}"))?;
    if !status.is_success() {
        return Err(format!(
            "inception auth request failed with status {}: {}",
            status.as_u16(),
            truncate_text(&body, 320)
        ));
    }
    let auth: InceptionGuestAuthResponse =
        serde_json::from_str(&body).map_err(|err| format!("inception auth parse failed: {err}"))?;
    Ok(normalize_optional_text(&auth.token, 8_192))
}

async fn invoke_inception_guest_model_direct(
    client: &Client,
    request: &WebsiteBridgeRequest<'_>,
    origin: &str,
    model: &str,
) -> Result<WebsiteBridgeResponse, String> {
    let mut completion_payload = request.request_overrides.clone();
    completion_payload.insert("model".to_owned(), Value::String(model.to_owned()));
    completion_payload.insert(
        "messages".to_owned(),
        Value::Array(request.messages.to_vec()),
    );
    completion_payload
        .entry("stream".to_owned())
        .or_insert(Value::Bool(false));
    if !request.tools.is_empty() {
        completion_payload.insert("tools".to_owned(), Value::Array(request.tools.to_vec()));
        completion_payload
            .entry("tool_choice".to_owned())
            .or_insert(Value::String("auto".to_owned()));
    }
    let mut attempts = Vec::new();
    let candidate_endpoints = [
        format!("{origin}/api/v1/chat/completions"),
        format!("{origin}/api/chat/completions"),
    ];
    for completion_endpoint in candidate_endpoints {
        let mut completion_request = client
            .post(&completion_endpoint)
            .header("Content-Type", "application/json")
            .header("X-Accel-Buffering", "no");
        for (name, value) in request.headers {
            completion_request = completion_request.header(name, value);
        }
        if let Some(api_key) = request
            .api_key
            .and_then(|value| normalize_optional_text(value, 8_192))
        {
            completion_request = completion_request.header(
                request.auth_header_name,
                format!("{}{}", request.auth_header_prefix, api_key),
            );
        }
        let completion_response = completion_request
            .json(&Value::Object(completion_payload.clone()))
            .send()
            .await
            .map_err(|err| format!("inception completion request failed: {err}"))?;
        let completion_status = resolve_effective_status(&completion_response);
        let completion_body = completion_response
            .text()
            .await
            .map_err(|err| format!("inception completion body read failed: {err}"))?;
        if !completion_status.is_success() {
            attempts.push(format!(
                "{completion_endpoint}: status={} body={}",
                completion_status.as_u16(),
                truncate_text(&completion_body, 640)
            ));
            continue;
        }
        let openai_body =
            parse_inception_response_to_openai_body(&completion_body).ok_or_else(|| {
                format!(
                    "inception completion response missing assistant content: {}",
                    truncate_text(&completion_body, 640)
                )
            })?;
        return Ok(WebsiteBridgeResponse {
            body: openai_body,
            endpoint: completion_endpoint,
        });
    }
    Err(format!(
        "inception direct completion failed; attempts: {}",
        attempts.join(" | ")
    ))
}

async fn invoke_inception_guest_model_legacy(
    client: &Client,
    request: &WebsiteBridgeRequest<'_>,
    origin: &str,
    auth_token: &str,
    model: &str,
) -> Result<WebsiteBridgeResponse, String> {
    let create_chat_endpoint = format!("{origin}/api/chats/new");
    let create_chat_payload = json!({
        "title": "OpenClaw Rust",
        "models": [model],
        "chat_mode": request
            .request_overrides
            .get("chat_mode")
            .cloned()
            .unwrap_or_else(|| Value::String("normal".to_owned())),
        "chat_type": request
            .request_overrides
            .get("chat_type")
            .cloned()
            .unwrap_or_else(|| Value::String("t2t".to_owned())),
        "timestamp": qwen_now_ms()
    });
    let mut create_chat_request = client
        .post(&create_chat_endpoint)
        .header("Authorization", format!("Bearer {}", auth_token))
        .header("Content-Type", "application/json");
    for (name, value) in request.headers {
        create_chat_request = create_chat_request.header(name, value);
    }
    let create_chat_response = create_chat_request
        .json(&create_chat_payload)
        .send()
        .await
        .map_err(|err| format!("inception chat create request failed: {err}"))?;
    let create_chat_status = resolve_effective_status(&create_chat_response);
    let create_chat_body = create_chat_response
        .text()
        .await
        .map_err(|err| format!("inception chat create body read failed: {err}"))?;
    if !create_chat_status.is_success() {
        return Err(format!(
            "inception chat create failed with status {}: {}",
            create_chat_status.as_u16(),
            truncate_text(&create_chat_body, 320)
        ));
    }
    let chat: InceptionChatCreateResponse = serde_json::from_str(&create_chat_body)
        .map_err(|err| format!("inception chat create parse failed: {err}"))?;
    let chat_id = normalize_optional_text(&chat.id, 512)
        .ok_or_else(|| "inception chat create response missing chat id".to_owned())?;

    let mut completion_payload = request.request_overrides.clone();
    completion_payload.insert("chat_id".to_owned(), Value::String(chat_id.clone()));
    completion_payload.insert("model".to_owned(), Value::String(model.to_owned()));
    completion_payload.insert(
        "messages".to_owned(),
        Value::Array(request.messages.to_vec()),
    );
    completion_payload
        .entry("version".to_owned())
        .or_insert(Value::String("2.1".to_owned()));
    completion_payload
        .entry("timestamp".to_owned())
        .or_insert(json!(qwen_now_ms()));
    completion_payload
        .entry("chat_mode".to_owned())
        .or_insert(Value::String("normal".to_owned()));
    completion_payload
        .entry("stream".to_owned())
        .or_insert(Value::Bool(false));
    if !request.tools.is_empty() {
        completion_payload.insert("tools".to_owned(), Value::Array(request.tools.to_vec()));
        completion_payload
            .entry("tool_choice".to_owned())
            .or_insert(Value::String("auto".to_owned()));
    }

    let completion_endpoint = format!("{origin}/api/chat/completions?chat_id={chat_id}");
    let mut completion_request = client
        .post(&completion_endpoint)
        .header("Authorization", format!("Bearer {}", auth_token))
        .header("Content-Type", "application/json")
        .header("X-Accel-Buffering", "no");
    for (name, value) in request.headers {
        completion_request = completion_request.header(name, value);
    }
    let completion_response = completion_request
        .json(&Value::Object(completion_payload))
        .send()
        .await
        .map_err(|err| format!("inception completion request failed: {err}"))?;
    let completion_status = resolve_effective_status(&completion_response);
    let completion_body = completion_response
        .text()
        .await
        .map_err(|err| format!("inception completion body read failed: {err}"))?;
    if !completion_status.is_success() {
        return Err(format!(
            "inception completion failed with status {}: {}",
            completion_status.as_u16(),
            truncate_text(&completion_body, 640)
        ));
    }
    let openai_body =
        parse_inception_response_to_openai_body(&completion_body).ok_or_else(|| {
            format!(
                "inception completion response missing assistant content: {}",
                truncate_text(&completion_body, 640)
            )
        })?;
    Ok(WebsiteBridgeResponse {
        body: openai_body,
        endpoint: completion_endpoint,
    })
}

async fn invoke_qwen_guest_bridge(
    client: &Client,
    request: &WebsiteBridgeRequest<'_>,
) -> Result<WebsiteBridgeResponse, String> {
    let origin = resolve_qwen_origin(request)
        .ok_or_else(|| "qwen bridge origin could not be resolved".to_owned())?;
    let model_candidates = build_qwen_model_candidates(request.model);
    if model_candidates.is_empty() {
        return Err("qwen bridge has no candidate models".to_owned());
    }
    let mut attempts = Vec::new();
    for model in &model_candidates {
        match invoke_qwen_guest_model_v2(client, request, &origin, model).await {
            Ok(response) => return Ok(response),
            Err(err) => attempts.push(format!("{model}: v2 {err}")),
        }
    }
    match resolve_qwen_auth_token(client, request, &origin).await {
        Ok(auth_token) => {
            for model in &model_candidates {
                match invoke_qwen_guest_model_v1(client, request, &origin, &auth_token, model).await
                {
                    Ok(response) => return Ok(response),
                    Err(err) => attempts.push(format!("{model}: v1 {err}")),
                }
            }
        }
        Err(err) => attempts.push(format!("v1 auth {err}")),
    }
    Err(format!(
        "qwen bridge exhausted model candidates; attempts: {}",
        attempts.join(" | ")
    ))
}

fn resolve_qwen_origin(request: &WebsiteBridgeRequest<'_>) -> Option<String> {
    request
        .website_url
        .and_then(normalize_origin_url)
        .or_else(|| {
            request
                .candidate_base_urls
                .iter()
                .find_map(|candidate| normalize_origin_url(candidate))
        })
        .or_else(|| Some("https://chat.qwen.ai".to_owned()))
}

async fn resolve_qwen_auth_token(
    client: &Client,
    request: &WebsiteBridgeRequest<'_>,
    origin: &str,
) -> Result<String, String> {
    if let Some(token) = request
        .api_key
        .and_then(|value| normalize_optional_text(value, 8_192))
    {
        return Ok(token);
    }
    let auth_endpoint = format!("{origin}/api/v1/auths/");
    let auth_response = client
        .get(&auth_endpoint)
        .send()
        .await
        .map_err(|err| format!("qwen auth request failed: {err}"))?;
    let status = resolve_effective_status(&auth_response);
    let body = auth_response
        .text()
        .await
        .map_err(|err| format!("qwen auth body read failed: {err}"))?;
    if !status.is_success() {
        return Err(format!(
            "qwen auth request failed with status {}: {}",
            status.as_u16(),
            truncate_text(&body, 320)
        ));
    }
    let auth: QwenGuestAuthResponse =
        serde_json::from_str(&body).map_err(|err| format!("qwen auth parse failed: {err}"))?;
    let token = normalize_optional_text(&auth.token, 8_192)
        .ok_or_else(|| "qwen auth response missing token".to_owned())?;
    Ok(token)
}

fn qwen_extract_primary_prompt(messages: &[Value]) -> Option<String> {
    extract_signature_prompt(messages)
}

fn qwen_build_v2_messages(model: &str, prompt: &str, chat_type: &str) -> Vec<Value> {
    let timestamp_secs = (qwen_now_ms() / 1_000) as u64;
    let message_id = format!(
        "qwen-msg-{timestamp_secs}-{}",
        ZAI_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    vec![json!({
        "fid": message_id,
        "parentId": Value::Null,
        "childrenIds": [],
        "role": "user",
        "content": prompt,
        "user_action": "chat",
        "files": [],
        "timestamp": timestamp_secs,
        "models": [model],
        "chat_type": chat_type,
        "feature_config": {
            "thinking_enabled": true,
            "output_schema": "phase",
            "research_mode": "normal",
            "auto_thinking": true,
            "thinking_format": "summary",
            "auto_search": true
        },
        "extra": {
            "meta": {
                "subChatType": chat_type
            }
        },
        "sub_chat_type": chat_type,
        "parent_id": Value::Null
    })]
}

async fn invoke_qwen_guest_model_v2(
    client: &Client,
    request: &WebsiteBridgeRequest<'_>,
    origin: &str,
    model: &str,
) -> Result<WebsiteBridgeResponse, String> {
    let chat_mode = request
        .request_overrides
        .get("chat_mode")
        .and_then(Value::as_str)
        .unwrap_or("guest");
    let chat_type = request
        .request_overrides
        .get("chat_type")
        .and_then(Value::as_str)
        .unwrap_or("t2t");
    let prompt = qwen_extract_primary_prompt(request.messages)
        .ok_or_else(|| "qwen bridge could not extract user prompt".to_owned())?;

    let create_chat_endpoint = format!("{origin}/api/v2/chats/new");
    let create_chat_payload = json!({
        "title": "OpenClaw Rust",
        "models": [model],
        "chat_mode": chat_mode,
        "chat_type": chat_type,
        "timestamp": qwen_now_ms(),
        "project_id": request
            .request_overrides
            .get("project_id")
            .and_then(Value::as_str)
            .unwrap_or("")
    });
    let mut create_chat_request = client
        .post(&create_chat_endpoint)
        .header("Content-Type", "application/json");
    for (name, value) in request.headers {
        create_chat_request = create_chat_request.header(name, value);
    }
    let create_chat_response = create_chat_request
        .json(&create_chat_payload)
        .send()
        .await
        .map_err(|err| format!("qwen v2 chat create request failed: {err}"))?;
    let create_chat_status = resolve_effective_status(&create_chat_response);
    let create_chat_body = create_chat_response
        .text()
        .await
        .map_err(|err| format!("qwen v2 chat create body read failed: {err}"))?;
    if !create_chat_status.is_success() {
        return Err(format!(
            "qwen v2 chat create failed with status {}: {}",
            create_chat_status.as_u16(),
            truncate_text(&create_chat_body, 320)
        ));
    }
    let chat_id = qwen_extract_chat_id(&create_chat_body)
        .map_err(|err| format!("qwen v2 chat create parse failed: {err}"))?;

    let mut completion_payload = request.request_overrides.clone();
    completion_payload.insert("chat_id".to_owned(), Value::String(chat_id.clone()));
    completion_payload.insert("model".to_owned(), Value::String(model.to_owned()));
    completion_payload.insert("chat_mode".to_owned(), Value::String(chat_mode.to_owned()));
    completion_payload.insert("parent_id".to_owned(), Value::Null);
    completion_payload.insert(
        "messages".to_owned(),
        Value::Array(qwen_build_v2_messages(model, &prompt, chat_type)),
    );
    completion_payload
        .entry("version".to_owned())
        .or_insert(Value::String("2.1".to_owned()));
    completion_payload
        .entry("incremental_output".to_owned())
        .or_insert(Value::Bool(true));
    completion_payload
        .entry("timestamp".to_owned())
        .or_insert(json!(qwen_now_ms()));
    completion_payload
        .entry("stream".to_owned())
        .or_insert(Value::Bool(true));
    if !request.tools.is_empty() {
        completion_payload.insert("tools".to_owned(), Value::Array(request.tools.to_vec()));
        completion_payload
            .entry("tool_choice".to_owned())
            .or_insert(Value::String("auto".to_owned()));
    }

    let completion_endpoint = format!("{origin}/api/v2/chat/completions?chat_id={chat_id}");
    let mut completion_request = client
        .post(&completion_endpoint)
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("X-Accel-Buffering", "no");
    for (name, value) in request.headers {
        completion_request = completion_request.header(name, value);
    }
    let completion_response = completion_request
        .json(&Value::Object(completion_payload))
        .send()
        .await
        .map_err(|err| format!("qwen v2 completion request failed: {err}"))?;
    let completion_status = resolve_effective_status(&completion_response);
    let completion_body = completion_response
        .text()
        .await
        .map_err(|err| format!("qwen v2 completion body read failed: {err}"))?;
    if !completion_status.is_success() {
        return Err(format!(
            "qwen v2 completion failed with status {}: {}",
            completion_status.as_u16(),
            truncate_text(&completion_body, 640)
        ));
    }
    let openai_body = parse_qwen_response_to_openai_body(&completion_body).ok_or_else(|| {
        format!(
            "qwen v2 completion response missing assistant content: {}",
            truncate_text(&completion_body, 640)
        )
    })?;
    Ok(WebsiteBridgeResponse {
        body: openai_body,
        endpoint: completion_endpoint,
    })
}

async fn invoke_qwen_guest_model_v1(
    client: &Client,
    request: &WebsiteBridgeRequest<'_>,
    origin: &str,
    auth_token: &str,
    model: &str,
) -> Result<WebsiteBridgeResponse, String> {
    let create_chat_endpoint = format!("{origin}/api/chats/new");
    let create_chat_payload = json!({
        "title": "OpenClaw Rust",
        "models": [model],
        "chat_mode": request
            .request_overrides
            .get("chat_mode")
            .cloned()
            .unwrap_or_else(|| Value::String("normal".to_owned())),
        "chat_type": request
            .request_overrides
            .get("chat_type")
            .cloned()
            .unwrap_or_else(|| Value::String("t2t".to_owned())),
        "timestamp": qwen_now_ms()
    });
    let mut create_chat_request = client
        .post(&create_chat_endpoint)
        .header("Authorization", format!("Bearer {}", auth_token))
        .header("Content-Type", "application/json");
    for (name, value) in request.headers {
        create_chat_request = create_chat_request.header(name, value);
    }
    let create_chat_response = create_chat_request
        .json(&create_chat_payload)
        .send()
        .await
        .map_err(|err| format!("qwen chat create request failed: {err}"))?;
    let create_chat_status = resolve_effective_status(&create_chat_response);
    let create_chat_body = create_chat_response
        .text()
        .await
        .map_err(|err| format!("qwen chat create body read failed: {err}"))?;
    if !create_chat_status.is_success() {
        return Err(format!(
            "qwen chat create failed with status {}: {}",
            create_chat_status.as_u16(),
            truncate_text(&create_chat_body, 320)
        ));
    }
    let chat_id = qwen_extract_chat_id(&create_chat_body)
        .map_err(|err| format!("qwen chat create parse failed: {err}"))?;

    let mut completion_payload = request.request_overrides.clone();
    completion_payload.insert("chat_id".to_owned(), Value::String(chat_id.clone()));
    completion_payload.insert("model".to_owned(), Value::String(model.to_owned()));
    completion_payload.insert(
        "messages".to_owned(),
        Value::Array(request.messages.to_vec()),
    );
    completion_payload
        .entry("version".to_owned())
        .or_insert(Value::String("2.1".to_owned()));
    completion_payload
        .entry("timestamp".to_owned())
        .or_insert(json!(qwen_now_ms()));
    completion_payload
        .entry("chat_mode".to_owned())
        .or_insert(Value::String("normal".to_owned()));
    completion_payload
        .entry("stream".to_owned())
        .or_insert(Value::Bool(false));
    if !request.tools.is_empty() {
        completion_payload.insert("tools".to_owned(), Value::Array(request.tools.to_vec()));
        completion_payload
            .entry("tool_choice".to_owned())
            .or_insert(Value::String("auto".to_owned()));
    }

    let completion_endpoint = format!("{origin}/api/chat/completions?chat_id={chat_id}");
    let mut completion_request = client
        .post(&completion_endpoint)
        .header("Authorization", format!("Bearer {}", auth_token))
        .header("Content-Type", "application/json")
        .header("X-Accel-Buffering", "no");
    for (name, value) in request.headers {
        completion_request = completion_request.header(name, value);
    }
    let completion_response = completion_request
        .json(&Value::Object(completion_payload))
        .send()
        .await
        .map_err(|err| format!("qwen completion request failed: {err}"))?;
    let completion_status = resolve_effective_status(&completion_response);
    let completion_body = completion_response
        .text()
        .await
        .map_err(|err| format!("qwen completion body read failed: {err}"))?;
    if !completion_status.is_success() {
        return Err(format!(
            "qwen completion failed with status {}: {}",
            completion_status.as_u16(),
            truncate_text(&completion_body, 640)
        ));
    }
    let openai_body = parse_qwen_response_to_openai_body(&completion_body).ok_or_else(|| {
        format!(
            "qwen completion response missing assistant content: {}",
            truncate_text(&completion_body, 640)
        )
    })?;
    Ok(WebsiteBridgeResponse {
        body: openai_body,
        endpoint: completion_endpoint,
    })
}

fn qwen_extract_chat_id(raw: &str) -> Result<String, String> {
    if let Ok(parsed) = serde_json::from_str::<QwenChatCreateResponse>(raw) {
        if let Some(chat_id) = normalize_optional_text(&parsed.id, 512) {
            return Ok(chat_id);
        }
    }
    let parsed: Value =
        serde_json::from_str(raw).map_err(|err| format!("invalid qwen chat create JSON: {err}"))?;
    let chat_id = parsed
        .pointer("/data/id")
        .and_then(Value::as_str)
        .or_else(|| parsed.pointer("/id").and_then(Value::as_str))
        .or_else(|| parsed.pointer("/chat/id").and_then(Value::as_str))
        .and_then(|value| normalize_optional_text(value, 512))
        .ok_or_else(|| "qwen chat create response missing chat id".to_owned())?;
    Ok(chat_id)
}

fn build_qwen_model_candidates(model: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(requested) = normalize_optional_text(model, 256) {
        out.push(requested.clone());
        if let Some((_, model_id)) = requested.split_once('/') {
            if let Some(model_id) = normalize_optional_text(model_id, 256) {
                out.push(model_id);
            }
        }
        let key = normalize_model_alias_key(&requested);
        if key.contains("qwen35") {
            out.push("qwen3.5-397b-a17b".to_owned());
            out.push("qwen3.5-plus".to_owned());
            out.push("qwen3.5-flash".to_owned());
        }
    }
    if out.is_empty() {
        out.push("qwen3.5-397b-a17b".to_owned());
        out.push("qwen3.5-plus".to_owned());
        out.push("qwen3.5-flash".to_owned());
    }
    let mut dedup = Vec::with_capacity(out.len());
    for candidate in out {
        if dedup
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(&candidate))
        {
            continue;
        }
        dedup.push(candidate);
    }
    dedup
}

fn build_inception_model_candidates(model: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(requested) = normalize_optional_text(model, 256) {
        out.push(requested.clone());
        if let Some((_, model_id)) = requested.split_once('/') {
            if let Some(model_id) = normalize_optional_text(model_id, 256) {
                out.push(model_id);
            }
        }
        let key = normalize_model_alias_key(&requested);
        if key.contains("mercury") {
            out.push("mercury-2".to_owned());
            out.push("mercury".to_owned());
        }
    }
    if out.is_empty() {
        out.push("mercury-2".to_owned());
        out.push("mercury".to_owned());
    }
    let mut dedup = Vec::with_capacity(out.len());
    for candidate in out {
        if dedup
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(&candidate))
        {
            continue;
        }
        dedup.push(candidate);
    }
    dedup
}

fn normalize_model_alias_key(model: &str) -> String {
    model
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect()
}

fn qwen_now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0)
}

fn parse_qwen_response_to_openai_body(raw: &str) -> Option<String> {
    if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
        if openai_response_has_usable_assistant_output(&parsed) {
            return Some(parsed.to_string());
        }
        if let Some(content) = extract_qwen_assistant_content_from_value(&parsed) {
            return Some(
                json!({
                    "choices": [
                        {
                            "message": {
                                "content": content
                            }
                        }
                    ]
                })
                .to_string(),
            );
        }
    }
    parse_qwen_sse_to_openai_body(raw)
}

fn parse_inception_response_to_openai_body(raw: &str) -> Option<String> {
    parse_qwen_response_to_openai_body(raw)
}

async fn invoke_chatgpt_web_bridge(
    client: &Client,
    request: &WebsiteBridgeRequest<'_>,
) -> Result<WebsiteBridgeResponse, String> {
    let origin = resolve_chatgpt_origin(request)
        .ok_or_else(|| "chatgpt web bridge origin could not be resolved".to_owned())?;
    let access_token = request
        .api_key
        .and_then(|value| normalize_optional_text(value, 8_192))
        .ok_or_else(|| "chatgpt web bridge requires access token".to_owned())?;
    let prompt = extract_signature_prompt(request.messages)
        .ok_or_else(|| "chatgpt web bridge could not extract user prompt".to_owned())?;
    let model_candidates = build_chatgpt_model_candidates(request.model);
    if model_candidates.is_empty() {
        return Err("chatgpt web bridge has no candidate models".to_owned());
    }

    let mut attempts = Vec::new();
    for model in model_candidates {
        let endpoint = format!("{origin}/backend-api/conversation");
        let user_message_id = chatgpt_next_id("msg-user");
        let parent_message_id = chatgpt_next_id("msg-parent");
        let websocket_request_id = chatgpt_next_id("req");
        let payload = json!({
            "action": "next",
            "messages": [
                {
                    "id": user_message_id,
                    "author": { "role": "user" },
                    "content": {
                        "content_type": "text",
                        "parts": [prompt]
                    }
                }
            ],
            "parent_message_id": parent_message_id,
            "model": model,
            "history_and_training_disabled": false,
            "timezone_offset_min": 0,
            "conversation_mode": { "kind": "primary_assistant" },
            "websocket_request_id": websocket_request_id
        });

        let mut request_builder = client
            .post(&endpoint)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Accept", "text/event-stream")
            .header("Content-Type", "application/json")
            .header("Origin", &origin)
            .header("Referer", format!("{origin}/"));
        for (name, value) in request.headers {
            request_builder = request_builder.header(name, value);
        }

        let response = match request_builder.json(&payload).send().await {
            Ok(value) => value,
            Err(err) => {
                attempts.push(format!("{model}: transport_error: {err}"));
                continue;
            }
        };
        let status = resolve_effective_status(&response);
        let body = match response.text().await {
            Ok(value) => value,
            Err(err) => {
                attempts.push(format!("{model}: body_read_error: {err}"));
                continue;
            }
        };
        if !status.is_success() {
            attempts.push(format!(
                "{model}: status={} body={}",
                status.as_u16(),
                truncate_text(&body, 320)
            ));
            continue;
        }
        let Some(openai_body) = parse_chatgpt_response_to_openai_body(&body) else {
            attempts.push(format!(
                "{model}: parse_error body={}",
                truncate_text(&body, 240)
            ));
            continue;
        };
        return Ok(WebsiteBridgeResponse {
            body: openai_body,
            endpoint: endpoint.clone(),
        });
    }

    Err(format!(
        "chatgpt web bridge exhausted model candidates; attempts: {}",
        attempts.join(" | ")
    ))
}

fn resolve_chatgpt_origin(request: &WebsiteBridgeRequest<'_>) -> Option<String> {
    for candidate in request.candidate_base_urls {
        if !chatgpt_url_hint_matches(candidate) {
            continue;
        }
        if let Some(origin) = normalize_origin_url(candidate) {
            return Some(origin);
        }
    }
    request
        .website_url
        .filter(|url| chatgpt_url_hint_matches(url))
        .and_then(normalize_origin_url)
}

fn build_chatgpt_model_candidates(model: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(requested) = normalize_optional_text(model, 256) {
        out.push(requested.clone());
        if let Some((_, bare_model)) = requested.split_once('/') {
            if let Some(bare_model) = normalize_optional_text(bare_model, 256) {
                out.push(bare_model);
            }
        }
        if let Some(normalized_browser_model) = normalize_chatgpt_browser_model_id(&requested) {
            out.push(normalized_browser_model);
        }
    }
    out.push("gpt-5-2".to_owned());
    out.push("gpt-5-1".to_owned());
    out.push("gpt-5".to_owned());
    out.push("gpt-5-mini".to_owned());
    out.push("gpt-5.2-thinking-extended".to_owned());
    out.push("gpt-4o".to_owned());

    let mut dedup = Vec::with_capacity(out.len());
    for model_id in out {
        if dedup
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(&model_id))
        {
            continue;
        }
        dedup.push(model_id);
    }
    dedup
}

fn normalize_chatgpt_browser_model_id(model: &str) -> Option<String> {
    let requested = normalize_optional_text(model, 256)?;
    let normalized = requested
        .split('/')
        .next_back()
        .unwrap_or(requested.as_str())
        .trim()
        .to_ascii_lowercase()
        .replace('_', "-");
    if normalized.is_empty() {
        return None;
    }
    if normalized.contains("gpt-5.2") || normalized.starts_with("gpt-5-2") {
        return Some("gpt-5-2".to_owned());
    }
    if normalized.contains("gpt-5.1") || normalized.starts_with("gpt-5-1") {
        return Some("gpt-5-1".to_owned());
    }
    if normalized.contains("gpt-5-mini") || normalized.contains("gpt-5mini") {
        return Some("gpt-5-mini".to_owned());
    }
    if normalized.starts_with("gpt-5") {
        return Some("gpt-5".to_owned());
    }
    if normalized.starts_with("gpt-4o") {
        return Some("gpt-4o".to_owned());
    }
    Some(requested)
}

fn parse_chatgpt_response_to_openai_body(raw: &str) -> Option<String> {
    if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
        if openai_response_has_usable_assistant_output(&parsed) {
            return Some(parsed.to_string());
        }
        if let Some(content) = extract_chatgpt_assistant_content_from_value(&parsed) {
            return Some(
                json!({
                    "choices": [
                        {
                            "message": {
                                "content": content
                            }
                        }
                    ]
                })
                .to_string(),
            );
        }
    }
    parse_chatgpt_sse_to_openai_body(raw)
}

fn extract_chatgpt_assistant_content_from_value(parsed: &Value) -> Option<String> {
    if let Some(parts) = parsed
        .pointer("/message/content/parts")
        .and_then(Value::as_array)
    {
        let text = parts
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(content) = normalize_optional_text(&cleanup_qwen_completion_text(&text), 12_000)
        {
            return Some(content);
        }
    }
    parsed
        .pointer("/message/content/text")
        .and_then(Value::as_str)
        .and_then(|value| normalize_optional_text(value, 12_000))
}

fn parse_chatgpt_sse_to_openai_body(raw: &str) -> Option<String> {
    let mut content = String::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("data:") {
            continue;
        }
        let payload = trimmed.trim_start_matches("data:").trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let parsed: Value = match serde_json::from_str(payload) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if let Some(delta) = extract_chatgpt_assistant_content_from_value(&parsed) {
            content = delta;
        }
    }
    let content = normalize_optional_text(&content, 12_000)?;
    Some(
        json!({
            "choices": [
                {
                    "message": {
                        "content": content
                    }
                }
            ]
        })
        .to_string(),
    )
}

fn chatgpt_next_id(prefix: &str) -> String {
    let seq = CHATGPT_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{seq}", chatgpt_now_ms())
}

fn chatgpt_now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0)
}

fn extract_qwen_assistant_content_from_value(parsed: &Value) -> Option<String> {
    let content = parsed
        .pointer("/data/content")
        .and_then(Value::as_str)
        .or_else(|| parsed.pointer("/message/content").and_then(Value::as_str))
        .or_else(|| parsed.pointer("/content").and_then(Value::as_str))
        .or_else(|| parsed.pointer("/response").and_then(Value::as_str))?;
    normalize_optional_text(&cleanup_qwen_completion_text(content), 12_000)
}

fn parse_qwen_sse_to_openai_body(raw: &str) -> Option<String> {
    let mut deltas = String::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("data:") {
            continue;
        }
        let payload = trimmed.trim_start_matches("data:").trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let parsed: Value = match serde_json::from_str(payload) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if let Some(delta) = parsed
            .pointer("/choices/0/delta/content")
            .and_then(Value::as_str)
        {
            deltas.push_str(delta);
            continue;
        }
        if let Some(delta) = parsed
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
        {
            deltas.push_str(delta);
            continue;
        }
        if let Some(delta) = parsed
            .pointer("/data/delta_content")
            .and_then(Value::as_str)
        {
            deltas.push_str(delta);
            continue;
        }
        if let Some(delta) = parsed.pointer("/data/content").and_then(Value::as_str) {
            deltas.push_str(delta);
        }
    }
    let cleaned = cleanup_qwen_completion_text(&deltas);
    let content = normalize_optional_text(&cleaned, 12_000)?;
    Some(
        json!({
            "choices": [
                {
                    "message": {
                        "content": content
                    }
                }
            ]
        })
        .to_string(),
    )
}

fn cleanup_qwen_completion_text(raw: &str) -> String {
    let normalized = raw.replace('\r', "");
    if let Some(idx) = normalized.rfind("</think>") {
        let tail = normalized[idx + "</think>".len()..].trim();
        if !tail.is_empty() {
            return tail.to_owned();
        }
    }
    normalized
        .replace("<think>", "")
        .replace("</think>", "")
        .trim()
        .to_owned()
}

fn normalize_origin_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed = Url::parse(trimmed)
        .or_else(|_| Url::parse(&format!("https://{trimmed}")))
        .ok()?;
    let host = parsed.host_str()?;
    let mut origin = format!("{}://{host}", parsed.scheme());
    if let Some(port) = parsed.port() {
        origin.push(':');
        origin.push_str(&port.to_string());
    }
    Some(origin)
}

fn extract_signature_prompt(messages: &[Value]) -> Option<String> {
    for message in messages.iter().rev() {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if role.eq_ignore_ascii_case("user") {
            if let Some(text) = extract_message_content_text(message.get("content")) {
                return normalize_optional_text(&text, 12_000);
            }
        }
    }
    for message in messages.iter().rev() {
        if let Some(text) = extract_message_content_text(message.get("content")) {
            return normalize_optional_text(&text, 12_000);
        }
    }
    None
}

fn extract_message_content_text(content: Option<&Value>) -> Option<String> {
    let content = content?;
    match content {
        Value::String(raw) => normalize_optional_text(raw, 12_000),
        Value::Array(items) => {
            let mut out = String::new();
            for item in items {
                let text = item
                    .get("text")
                    .and_then(Value::as_str)
                    .or_else(|| item.pointer("/text/value").and_then(Value::as_str))
                    .or_else(|| item.pointer("/content").and_then(Value::as_str));
                if let Some(text) = text {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(text.trim());
                }
            }
            normalize_optional_text(&out, 12_000)
        }
        Value::Object(object) => object
            .get("text")
            .and_then(Value::as_str)
            .and_then(|raw| normalize_optional_text(raw, 12_000))
            .or_else(|| {
                object
                    .get("content")
                    .and_then(Value::as_str)
                    .and_then(|raw| normalize_optional_text(raw, 12_000))
            }),
        Value::Null => None,
        _ => normalize_optional_text(&content.to_string(), 12_000),
    }
}

fn zai_compute_signature(
    request_id: &str,
    user_id: &str,
    signature_prompt: &str,
    timestamp_ms: &str,
) -> String {
    let mut payload_entries = vec![
        ("timestamp", timestamp_ms.to_owned()),
        ("requestId", request_id.to_owned()),
        ("user_id", user_id.to_owned()),
    ];
    payload_entries.sort_by(|a, b| a.0.cmp(b.0));
    let sorted_payload = payload_entries
        .into_iter()
        .map(|(name, value)| format!("{name},{value}"))
        .collect::<Vec<_>>()
        .join(",");
    let base64_prompt = BASE64_STANDARD.encode(signature_prompt.as_bytes());
    let bucket = timestamp_ms
        .parse::<u128>()
        .map(|value| value / 300_000)
        .unwrap_or(0);
    let rolling_key = zai_hmac_hex(ZAI_SIGNING_KEY.as_bytes(), bucket.to_string().as_bytes());
    let signed = format!("{sorted_payload}|{base64_prompt}|{timestamp_ms}");
    zai_hmac_hex(rolling_key.as_bytes(), signed.as_bytes())
}

fn zai_hmac_hex(key: &[u8], payload: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key)
        .expect("HMAC-SHA256 supports arbitrary key sizes for zai bridge");
    mac.update(payload);
    let bytes = mac.finalize().into_bytes();
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn zai_next_id(prefix: &str) -> String {
    let seq = ZAI_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{seq}", zai_now_ms())
}

fn zai_now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0)
}

fn parse_zai_sse_to_openai_body(raw: &str) -> Option<String> {
    let mut deltas = String::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("data:") {
            continue;
        }
        let payload = trimmed.trim_start_matches("data:").trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let parsed: Value = match serde_json::from_str(payload) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if parsed
            .pointer("/type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            != "chat:completion"
        {
            continue;
        }
        if let Some(delta) = parsed
            .pointer("/data/delta_content")
            .and_then(Value::as_str)
        {
            deltas.push_str(delta);
        } else if let Some(delta) = parsed.pointer("/data/content").and_then(Value::as_str) {
            deltas.push_str(delta);
        }
    }
    let cleaned = cleanup_zai_completion_text(&deltas);
    let content = normalize_optional_text(&cleaned, 12_000)?;
    Some(
        json!({
            "choices": [
                {
                    "message": {
                        "content": content
                    }
                }
            ]
        })
        .to_string(),
    )
}

fn cleanup_zai_completion_text(raw: &str) -> String {
    let normalized = raw.replace('\r', "");
    if let Some(idx) = normalized.rfind("</details>") {
        let tail = normalized[idx + "</details>".len()..].trim();
        if !tail.is_empty() {
            return tail.to_owned();
        }
    }
    let stripped = normalized
        .replace("<details type=\"reasoning\" done=\"false\">", "")
        .replace("</details>", "");
    let filtered = stripped
        .lines()
        .filter(|line| !line.trim_start().starts_with('>'))
        .collect::<Vec<_>>()
        .join("\n");
    if filtered.trim().is_empty() {
        stripped.trim().to_owned()
    } else {
        filtered.trim().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn bridge_falls_back_to_second_candidate_after_error() {
        let bad_listener = TcpListener::bind("127.0.0.1:0").expect("bind bad listener");
        let bad_addr = bad_listener.local_addr().expect("bad addr");
        let bad_server = std::thread::spawn(move || {
            let (mut stream, _) = bad_listener.accept().expect("accept bad request");
            let mut buffer = vec![0_u8; 16 * 1024];
            let _ = stream.read(&mut buffer).expect("read bad request");
            let body = "{\"error\":\"forbidden\"}";
            let response = format!(
                "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write bad response");
        });

        let good_listener = TcpListener::bind("127.0.0.1:0").expect("bind good listener");
        let good_addr = good_listener.local_addr().expect("good addr");
        let good_server = std::thread::spawn(move || {
            let (mut stream, _) = good_listener.accept().expect("accept good request");
            let mut buffer = vec![0_u8; 32 * 1024];
            let _ = stream.read(&mut buffer).expect("read good request");
            let body = json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "ok"
                        }
                    }
                ]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write good response");
        });

        let candidates = vec![
            format!("http://{bad_addr}/v1"),
            format!("http://{good_addr}/v1"),
        ];
        let messages = vec![json!({
            "role": "user",
            "content": "ok?"
        })];
        let request_overrides = serde_json::Map::new();
        let request = WebsiteBridgeRequest {
            provider: "bridge-test",
            model: "free-test",
            messages: &messages,
            tools: &[],
            timeout_ms: 30_000,
            website_url: None,
            candidate_base_urls: &candidates,
            headers: &[],
            auth_header_name: "Authorization",
            auth_header_prefix: "Bearer ",
            api_key: None,
            request_overrides: &request_overrides,
        };
        let response = invoke_openai_compatible(request)
            .await
            .expect("bridge response");
        assert!(response.endpoint.contains(&good_addr.to_string()));

        bad_server.join().expect("join bad server");
        good_server.join().expect("join good server");
    }

    #[tokio::test]
    async fn bridge_skips_empty_assistant_content_until_coherent_reply() {
        let bad_listener = TcpListener::bind("127.0.0.1:0").expect("bind bad listener");
        let bad_addr = bad_listener.local_addr().expect("bad addr");
        let bad_server = std::thread::spawn(move || {
            let (mut stream, _) = bad_listener.accept().expect("accept bad request");
            let mut buffer = vec![0_u8; 16 * 1024];
            let _ = stream.read(&mut buffer).expect("read bad request");
            let body = json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": ""
                        }
                    }
                ]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write bad response");
        });

        let good_listener = TcpListener::bind("127.0.0.1:0").expect("bind good listener");
        let good_addr = good_listener.local_addr().expect("good addr");
        let good_server = std::thread::spawn(move || {
            let (mut stream, _) = good_listener.accept().expect("accept good request");
            let mut buffer = vec![0_u8; 32 * 1024];
            let _ = stream.read(&mut buffer).expect("read good request");
            let body = json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "ok"
                        }
                    }
                ]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write good response");
        });

        let candidates = vec![
            format!("http://{bad_addr}/v1"),
            format!("http://{good_addr}/v1"),
        ];
        let messages = vec![json!({
            "role": "user",
            "content": "ok?"
        })];
        let request_overrides = serde_json::Map::new();
        let request = WebsiteBridgeRequest {
            provider: "bridge-test",
            model: "free-test",
            messages: &messages,
            tools: &[],
            timeout_ms: 30_000,
            website_url: None,
            candidate_base_urls: &candidates,
            headers: &[],
            auth_header_name: "Authorization",
            auth_header_prefix: "Bearer ",
            api_key: None,
            request_overrides: &request_overrides,
        };
        let response = invoke_openai_compatible(request)
            .await
            .expect("bridge response");
        assert!(response.endpoint.contains(&good_addr.to_string()));

        bad_server.join().expect("join bad server");
        good_server.join().expect("join good server");
    }

    #[tokio::test]
    async fn bridge_does_not_send_auth_header_without_api_key() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let addr = listener.local_addr().expect("addr");
        let captured = Arc::new(Mutex::new(String::new()));
        let captured_server = Arc::clone(&captured);
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut buffer = vec![0_u8; 64 * 1024];
            let read = stream.read(&mut buffer).expect("read request");
            let request_text = String::from_utf8_lossy(&buffer[..read]).to_string();
            if let Ok(mut guard) = captured_server.lock() {
                *guard = request_text;
            }
            let body = json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "ok"
                        }
                    }
                ]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        });

        let candidates = vec![format!("http://{addr}/v1")];
        let messages = vec![json!({
            "role": "user",
            "content": "ok?"
        })];
        let request_overrides = serde_json::Map::new();
        let request = WebsiteBridgeRequest {
            provider: "bridge-test",
            model: "free-test",
            messages: &messages,
            tools: &[],
            timeout_ms: 30_000,
            website_url: None,
            candidate_base_urls: &candidates,
            headers: &[],
            auth_header_name: "Authorization",
            auth_header_prefix: "Bearer ",
            api_key: None,
            request_overrides: &request_overrides,
        };
        let _ = invoke_openai_compatible(request)
            .await
            .expect("bridge response");
        server.join().expect("join server");

        let request_text = captured.lock().expect("lock captured").clone();
        assert!(
            !request_text
                .to_ascii_lowercase()
                .contains("\r\nauthorization:"),
            "bridge request should not send Authorization header without api key"
        );
    }

    #[tokio::test]
    async fn bridge_parallel_stress_requests_succeed() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let addr = listener.local_addr().expect("listener addr");
        let request_count = 24usize;
        let server = std::thread::spawn(move || {
            for _ in 0..request_count {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut buffer = vec![0_u8; 32 * 1024];
                let _ = stream.read(&mut buffer).expect("read request");
                let body = json!({
                    "choices": [
                        {
                            "message": {
                                "role": "assistant",
                                "content": "ok"
                            }
                        }
                    ]
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
            }
        });

        let mut tasks = Vec::new();
        for _ in 0..request_count {
            let candidate = format!("http://{addr}/v1");
            tasks.push(tokio::spawn(async move {
                let candidates = vec![candidate];
                let messages = vec![json!({
                    "role": "user",
                    "content": "ping"
                })];
                let request_overrides = serde_json::Map::new();
                let request = WebsiteBridgeRequest {
                    provider: "bridge-stress",
                    model: "stress-model",
                    messages: &messages,
                    tools: &[],
                    timeout_ms: 30_000,
                    website_url: None,
                    candidate_base_urls: &candidates,
                    headers: &[],
                    auth_header_name: "Authorization",
                    auth_header_prefix: "Bearer ",
                    api_key: None,
                    request_overrides: &request_overrides,
                };
                let response = invoke_openai_compatible(request)
                    .await
                    .expect("stress bridge response");
                assert!(response.body.contains("\"choices\""));
            }));
        }
        for task in tasks {
            task.await.expect("join stress task");
        }
        server.join().expect("join stress server");
    }

    #[test]
    fn zai_signature_matches_known_browser_vector() {
        let signature = zai_compute_signature(
            "d9fb3ca8-676f-46c3-90c7-ddc70a74e46d",
            "d0d30815-b85a-4270-af63-b5346a9f3b13",
            "Reply with exactly: ok",
            "1771762969397",
        );
        assert_eq!(
            signature,
            "26be53ded6069ead38e6d4d596e54b020aaa2e17f75a4fa29325ead00484b214"
        );
    }

    #[test]
    fn zai_sse_payload_converts_to_openai_shape() {
        let sse = r#"data: {"type":"chat:completion","data":{"delta_content":"<details type=\"reasoning\" done=\"false\">debug</details>","phase":"thinking"}}

data: {"type":"chat:completion","data":{"delta_content":"ok","phase":"done"}}"#;
        let parsed = parse_zai_sse_to_openai_body(sse).expect("parse zai sse");
        let json: Value = serde_json::from_str(&parsed).expect("openai json");
        assert_eq!(
            json.pointer("/choices/0/message/content")
                .and_then(Value::as_str),
            Some("ok")
        );
    }

    #[test]
    fn zai_model_candidates_include_glm_fallback_set() {
        let candidates = build_zai_model_candidates("zai/glm-5-web-free");
        assert_eq!(
            candidates.first().map(String::as_str),
            Some("zai/glm-5-web-free")
        );
        assert!(candidates.iter().any(|item| item == "glm-5"));
        assert!(candidates.iter().any(|item| item == "glm-5-air"));
    }

    #[tokio::test]
    async fn official_zai_guest_bridge_path_generates_openai_response() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let addr = listener.local_addr().expect("listener addr");
        let captured_completion = Arc::new(Mutex::new(String::new()));
        let captured_completion_server = Arc::clone(&captured_completion);

        let server = std::thread::spawn(move || {
            let mut completion_handled = false;
            while !completion_handled {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut buffer = vec![0_u8; 128 * 1024];
                let read = stream.read(&mut buffer).expect("read request");
                let request_text = String::from_utf8_lossy(&buffer[..read]).to_string();
                let request_line = request_text.lines().next().unwrap_or_default().to_owned();
                if request_line.starts_with("GET / HTTP/1.1")
                    || request_line.starts_with("GET / HTTP/1.0")
                {
                    let body = "ok";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write root response");
                    continue;
                }
                if request_line.contains("/api/v1/auths/") {
                    let body = json!({
                        "id": "guest-123",
                        "name": "Guest-123",
                        "token": "guest-token-123"
                    })
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write auth response");
                    continue;
                }
                if request_line.contains("/api/v1/chats/new") {
                    let body = json!({
                        "id": "chat-123"
                    })
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write chat response");
                    continue;
                }
                if request_line.contains("/api/v2/chat/completions") {
                    if let Ok(mut guard) = captured_completion_server.lock() {
                        *guard = request_text.clone();
                    }
                    let body = "data: {\"type\":\"chat:completion\",\"data\":{\"delta_content\":\"<details type=\\\"reasoning\\\" done=\\\"false\\\">trace</details>\",\"phase\":\"thinking\"}}\n\ndata: {\"type\":\"chat:completion\",\"data\":{\"delta_content\":\"ok\",\"phase\":\"done\"}}\n\n";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write completion response");
                    completion_handled = true;
                    continue;
                }
                let body = "{\"detail\":\"not found\"}";
                let response = format!(
                    "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write not found response");
            }
        });

        let base = format!("http://{addr}");
        let candidates: Vec<String> = Vec::new();
        let messages = vec![json!({
            "role": "user",
            "content": "Reply with exactly: ok"
        })];
        let request_overrides = serde_json::Map::new();
        let request = WebsiteBridgeRequest {
            provider: "zhipuai",
            model: "glm-5",
            messages: &messages,
            tools: &[],
            timeout_ms: 30_000,
            website_url: Some(&base),
            candidate_base_urls: &candidates,
            headers: &[],
            auth_header_name: "Authorization",
            auth_header_prefix: "Bearer ",
            api_key: None,
            request_overrides: &request_overrides,
        };
        let response = invoke_openai_compatible(request)
            .await
            .expect("official zai bridge response");
        let response_json: Value = serde_json::from_str(&response.body).expect("response json");
        assert_eq!(
            response_json
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str),
            Some("ok")
        );
        assert!(response.endpoint.contains("/api/v2/chat/completions?"));

        let completion_request = captured_completion.lock().expect("lock completion").clone();
        assert!(completion_request.contains("\r\nx-signature:"));
        assert!(completion_request.contains("\r\nauthorization: Bearer guest-token-123"));

        server.join().expect("join server");
    }

    #[tokio::test]
    async fn official_zai_guest_bridge_fallbacks_after_endpoint_failure_with_api_key() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let addr = listener.local_addr().expect("listener addr");

        let server = std::thread::spawn(move || {
            let mut completion_handled = false;
            while !completion_handled {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut buffer = vec![0_u8; 128 * 1024];
                let read = stream.read(&mut buffer).expect("read request");
                let request_text = String::from_utf8_lossy(&buffer[..read]).to_string();
                let request_line = request_text.lines().next().unwrap_or_default().to_owned();
                if request_line.starts_with("GET / HTTP/1.1")
                    || request_line.starts_with("GET / HTTP/1.0")
                {
                    let body = "ok";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write root response");
                    continue;
                }
                if request_line.contains("/api/v1/auths/") {
                    let body = json!({
                        "id": "guest-123",
                        "name": "Guest-123",
                        "token": "guest-token-123"
                    })
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write auth response");
                    continue;
                }
                if request_line.contains("/api/v1/chats/new") {
                    let body = json!({
                        "id": "chat-123"
                    })
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write chat response");
                    continue;
                }
                if request_line.contains("/api/v2/chat/completions") {
                    let body = "data: {\"type\":\"chat:completion\",\"data\":{\"delta_content\":\"ok\",\"phase\":\"done\"}}\n\n";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write completion response");
                    completion_handled = true;
                    continue;
                }
                let body = "{\"detail\":\"not found\"}";
                let response = format!(
                    "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write not found response");
            }
        });

        let base = format!("http://{addr}");
        let candidates = vec!["http://127.0.0.1:9/v1".to_owned()];
        let messages = vec![json!({
            "role": "user",
            "content": "Reply with exactly: ok"
        })];
        let request_overrides = serde_json::Map::new();
        let request = WebsiteBridgeRequest {
            provider: "zai",
            model: "glm-5",
            messages: &messages,
            tools: &[],
            timeout_ms: 30_000,
            website_url: Some(&base),
            candidate_base_urls: &candidates,
            headers: &[],
            auth_header_name: "Authorization",
            auth_header_prefix: "Bearer ",
            api_key: Some("stale-local-key"),
            request_overrides: &request_overrides,
        };
        let response = invoke_openai_compatible(request)
            .await
            .expect("zai bridge fallback response");
        let response_json: Value = serde_json::from_str(&response.body).expect("response json");
        assert_eq!(
            response_json
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str),
            Some("ok")
        );
        assert!(response.endpoint.contains("/api/v2/chat/completions?"));

        server.join().expect("join server");
    }

    #[test]
    fn qwen_model_candidates_include_qwen35_fallback_set() {
        let candidates = build_qwen_model_candidates("qwen3.5");
        assert_eq!(candidates.first().map(String::as_str), Some("qwen3.5"));
        assert!(candidates.iter().any(|item| item == "qwen3.5-397b-a17b"));
        assert!(candidates.iter().any(|item| item == "qwen3.5-plus"));
        assert!(candidates.iter().any(|item| item == "qwen3.5-flash"));

        let provider_prefixed = build_qwen_model_candidates("qwen-portal/qwen3.5-plus");
        assert!(provider_prefixed.iter().any(|item| item == "qwen3.5-plus"));
        assert!(provider_prefixed
            .iter()
            .any(|item| item == "qwen3.5-397b-a17b"));
    }

    #[test]
    fn endpoint_targets_loopback_identifies_local_bridge_hosts() {
        assert!(endpoint_targets_loopback(
            "http://127.0.0.1:43010/v1/chat/completions"
        ));
        assert!(endpoint_targets_loopback(
            "http://localhost:43010/v1/chat/completions"
        ));
        assert!(endpoint_targets_loopback(
            "http://[::1]:43010/v1/chat/completions"
        ));
        assert!(!endpoint_targets_loopback(
            "https://chat.qwen.ai/api/v2/chat/completions"
        ));
    }

    #[test]
    fn inception_model_candidates_include_mercury_fallback_set() {
        let candidates = build_inception_model_candidates("inception/mercury");
        assert_eq!(
            candidates.first().map(String::as_str),
            Some("inception/mercury")
        );
        assert!(candidates.iter().any(|item| item == "mercury-2"));
        assert!(candidates.iter().any(|item| item == "mercury"));
    }

    #[test]
    fn qwen_sse_payload_converts_to_openai_shape() {
        let sse = r#"data: {"choices":[{"delta":{"content":"<think>trace</think>"}}]}

data: {"choices":[{"delta":{"content":"ok"}}]}

data: [DONE]"#;
        let parsed = parse_qwen_sse_to_openai_body(sse).expect("parse qwen sse");
        let json: Value = serde_json::from_str(&parsed).expect("openai json");
        assert_eq!(
            json.pointer("/choices/0/message/content")
                .and_then(Value::as_str),
            Some("ok")
        );
    }

    #[test]
    fn chatgpt_sse_payload_converts_to_openai_shape() {
        let sse = r#"data: {"message":{"author":{"role":"assistant"},"content":{"parts":["thinking"]}}}

data: {"message":{"author":{"role":"assistant"},"content":{"parts":["final answer"]}}}

data: [DONE]"#;
        let parsed = parse_chatgpt_sse_to_openai_body(sse).expect("parse chatgpt sse");
        let json: Value = serde_json::from_str(&parsed).expect("openai json");
        assert_eq!(
            json.pointer("/choices/0/message/content")
                .and_then(Value::as_str),
            Some("final answer")
        );
    }

    #[test]
    fn chatgpt_model_candidates_include_extended_fallback() {
        let candidates = build_chatgpt_model_candidates("openai/gpt-5.2-thinking-extended");
        assert_eq!(
            candidates.first().map(String::as_str),
            Some("openai/gpt-5.2-thinking-extended")
        );
        assert!(candidates
            .iter()
            .any(|item| item.eq_ignore_ascii_case("gpt-5-2")));
        assert!(candidates
            .iter()
            .any(|item| item.eq_ignore_ascii_case("gpt-5.2-thinking-extended")));
        assert!(candidates
            .iter()
            .any(|item| item.eq_ignore_ascii_case("gpt-5-1")));
        assert!(candidates
            .iter()
            .any(|item| item.eq_ignore_ascii_case("gpt-5")));
        assert!(candidates
            .iter()
            .any(|item| item.eq_ignore_ascii_case("gpt-5-mini")));
        assert!(candidates
            .iter()
            .any(|item| item.eq_ignore_ascii_case("gpt-4o")));
    }

    #[test]
    fn chatgpt_model_candidates_normalize_browser_mode_aliases() {
        let candidates = build_chatgpt_model_candidates("openai/gpt-5.2-thinking");
        assert!(candidates
            .iter()
            .any(|item| item.eq_ignore_ascii_case("gpt-5-2")));

        let candidates = build_chatgpt_model_candidates("gpt-5-mini");
        assert!(candidates
            .iter()
            .any(|item| item.eq_ignore_ascii_case("gpt-5-mini")));

        let candidates = build_chatgpt_model_candidates("gpt-5.1-pro");
        assert!(candidates
            .iter()
            .any(|item| item.eq_ignore_ascii_case("gpt-5-1")));
    }

    #[tokio::test]
    async fn official_qwen_guest_bridge_path_generates_openai_response() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let addr = listener.local_addr().expect("listener addr");
        let captured_completion = Arc::new(Mutex::new(String::new()));
        let captured_completion_server = Arc::clone(&captured_completion);

        let server = std::thread::spawn(move || {
            let mut completion_handled = false;
            while !completion_handled {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut buffer = vec![0_u8; 128 * 1024];
                let read = stream.read(&mut buffer).expect("read request");
                let request_text = String::from_utf8_lossy(&buffer[..read]).to_string();
                let request_line = request_text.lines().next().unwrap_or_default().to_owned();
                if request_line.starts_with("GET / HTTP/1.1")
                    || request_line.starts_with("GET / HTTP/1.0")
                {
                    let body = "ok";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write root response");
                    continue;
                }
                if request_line.contains("/api/v1/auths/") {
                    let body = json!({
                        "token": "qwen-guest-token"
                    })
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write auth response");
                    continue;
                }
                if request_line.contains("/api/chats/new") {
                    let body = json!({
                        "id": "qwen-chat-123"
                    })
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write chat response");
                    continue;
                }
                if request_line.contains("/api/chat/completions") {
                    if let Ok(mut guard) = captured_completion_server.lock() {
                        *guard = request_text.clone();
                    }
                    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"<think>trace</think>\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write completion response");
                    completion_handled = true;
                    continue;
                }
                let body = "{\"detail\":\"not found\"}";
                let response = format!(
                    "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write not found response");
            }
        });

        let base = format!("http://{addr}");
        let candidates = vec![base.clone()];
        let messages = vec![json!({
            "role": "user",
            "content": "Reply with exactly: ok"
        })];
        let request_overrides = serde_json::Map::new();
        let request = WebsiteBridgeRequest {
            provider: "qwen-portal",
            model: "qwen3.5",
            messages: &messages,
            tools: &[],
            timeout_ms: 30_000,
            website_url: Some(&base),
            candidate_base_urls: &candidates,
            headers: &[],
            auth_header_name: "Authorization",
            auth_header_prefix: "Bearer ",
            api_key: None,
            request_overrides: &request_overrides,
        };
        let response = invoke_openai_compatible(request)
            .await
            .expect("official qwen bridge response");
        let response_json: Value = serde_json::from_str(&response.body).expect("response json");
        assert_eq!(
            response_json
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str),
            Some("ok")
        );
        assert!(response.endpoint.contains("/api/chat/completions?chat_id="));

        let completion_request = captured_completion.lock().expect("lock completion").clone();
        assert!(completion_request.contains("\r\nauthorization: Bearer qwen-guest-token"));

        server.join().expect("join server");
    }

    #[tokio::test]
    async fn official_inception_guest_bridge_path_generates_openai_response() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let addr = listener.local_addr().expect("listener addr");
        let captured_completion = Arc::new(Mutex::new(String::new()));
        let captured_completion_server = Arc::clone(&captured_completion);

        let server = std::thread::spawn(move || {
            let mut completion_handled = false;
            while !completion_handled {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut buffer = vec![0_u8; 128 * 1024];
                let read = stream.read(&mut buffer).expect("read request");
                let request_text = String::from_utf8_lossy(&buffer[..read]).to_string();
                let request_line = request_text.lines().next().unwrap_or_default().to_owned();
                if request_line.starts_with("GET / HTTP/1.1")
                    || request_line.starts_with("GET / HTTP/1.0")
                {
                    let body = "ok";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write root response");
                    continue;
                }
                if request_line.contains("/api/v1/auths/") {
                    let body = json!({
                        "token": "inception-guest-token"
                    })
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write auth response");
                    continue;
                }
                if request_line.contains("/api/chats/new") {
                    let body = json!({
                        "id": "inception-chat-123"
                    })
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write chat response");
                    continue;
                }
                if request_line.contains("/api/v1/chat/completions")
                    || request_line.contains("/api/chat/completions")
                {
                    if let Ok(mut guard) = captured_completion_server.lock() {
                        *guard = request_text.clone();
                    }
                    let body = json!({
                        "choices": [
                            {
                                "message": {
                                    "content": "ok"
                                }
                            }
                        ]
                    })
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write completion response");
                    completion_handled = true;
                    continue;
                }
                let body = "{\"detail\":\"not found\"}";
                let response = format!(
                    "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write not found response");
            }
        });

        let base = format!("http://{addr}");
        let candidates = vec![base.clone()];
        let messages = vec![json!({
            "role": "user",
            "content": "Reply with exactly: ok"
        })];
        let request_overrides = serde_json::Map::new();
        let request = WebsiteBridgeRequest {
            provider: "inception",
            model: "mercury-2",
            messages: &messages,
            tools: &[],
            timeout_ms: 30_000,
            website_url: Some(&base),
            candidate_base_urls: &candidates,
            headers: &[],
            auth_header_name: "Authorization",
            auth_header_prefix: "Bearer ",
            api_key: None,
            request_overrides: &request_overrides,
        };
        let response = invoke_openai_compatible(request)
            .await
            .expect("official inception bridge response");
        let response_json: Value = serde_json::from_str(&response.body).expect("response json");
        assert_eq!(
            response_json
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str),
            Some("ok")
        );
        assert!(response.endpoint.contains("/api/v1/chat/completions"));

        let completion_request = captured_completion.lock().expect("lock completion").clone();
        assert!(completion_request.contains("\"model\":\"mercury-2\""));

        server.join().expect("join server");
    }

    #[tokio::test]
    async fn official_inception_guest_bridge_fallbacks_after_endpoint_failure_with_api_key() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let addr = listener.local_addr().expect("listener addr");

        let server = std::thread::spawn(move || {
            let mut completion_handled = false;
            while !completion_handled {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut buffer = vec![0_u8; 128 * 1024];
                let read = stream.read(&mut buffer).expect("read request");
                let request_text = String::from_utf8_lossy(&buffer[..read]).to_string();
                let request_line = request_text.lines().next().unwrap_or_default().to_owned();
                if request_line.starts_with("GET / HTTP/1.1")
                    || request_line.starts_with("GET / HTTP/1.0")
                {
                    let body = "ok";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write root response");
                    continue;
                }
                if request_line.contains("/api/v1/auths/") {
                    let body = json!({
                        "token": "inception-guest-token"
                    })
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write auth response");
                    continue;
                }
                if request_line.contains("/api/chats/new") {
                    let body = json!({
                        "id": "inception-chat-123"
                    })
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write chat response");
                    continue;
                }
                if request_line.contains("/api/v1/chat/completions")
                    || request_line.contains("/api/chat/completions")
                {
                    let body = json!({
                        "choices": [
                            {
                                "message": {
                                    "content": "ok"
                                }
                            }
                        ]
                    })
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write completion response");
                    completion_handled = true;
                    continue;
                }
                let body = "{\"detail\":\"not found\"}";
                let response = format!(
                    "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write not found response");
            }
        });

        let base = format!("http://{addr}");
        let candidates = vec!["http://127.0.0.1:9/v1".to_owned()];
        let messages = vec![json!({
            "role": "user",
            "content": "Reply with exactly: ok"
        })];
        let request_overrides = serde_json::Map::new();
        let request = WebsiteBridgeRequest {
            provider: "inception",
            model: "mercury-2",
            messages: &messages,
            tools: &[],
            timeout_ms: 30_000,
            website_url: Some(&base),
            candidate_base_urls: &candidates,
            headers: &[],
            auth_header_name: "Authorization",
            auth_header_prefix: "Bearer ",
            api_key: Some("stale-local-key"),
            request_overrides: &request_overrides,
        };
        let response = invoke_openai_compatible(request)
            .await
            .expect("inception bridge fallback response");
        let response_json: Value = serde_json::from_str(&response.body).expect("response json");
        assert_eq!(
            response_json
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str),
            Some("ok")
        );
        assert!(response.endpoint.contains("/api/v1/chat/completions"));

        server.join().expect("join server");
    }
}
