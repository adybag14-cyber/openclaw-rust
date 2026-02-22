use std::time::Duration;

use reqwest::Client;
use serde_json::Value;

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

pub async fn invoke_openai_compatible(
    request: WebsiteBridgeRequest<'_>,
) -> Result<WebsiteBridgeResponse, String> {
    let timeout = Duration::from_millis(request.timeout_ms.max(1_000));
    let client = Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|err| format!("failed creating website bridge client: {err}"))?;

    let website_online = probe_website_status(&client, request.website_url).await;

    let mut candidate_endpoints = request
        .candidate_base_urls
        .iter()
        .filter_map(|raw| normalize_optional_text(raw, 2_048))
        .map(|base| resolve_chat_completion_endpoint(&base))
        .collect::<Vec<_>>();
    candidate_endpoints.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    if candidate_endpoints.is_empty() {
        return Err("website bridge has no candidate endpoints configured".to_owned());
    }

    let mut attempts = Vec::new();
    for endpoint in candidate_endpoints {
        let mut request_builder = client
            .post(&endpoint)
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
        let status = response.status();
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
        if parsed
            .pointer("/choices")
            .and_then(Value::as_array)
            .map(|choices| !choices.is_empty())
            .unwrap_or(false)
        {
            return Ok(WebsiteBridgeResponse { body, endpoint });
        }
        attempts.push(format!(
            "{endpoint}: missing_choices body={}",
            truncate_text(&parsed.to_string(), 240)
        ));
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
}
