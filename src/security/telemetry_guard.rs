use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::config::Config;

const TELEMETRY_CACHE_WINDOW_MS: u64 = 2_000;
const TELEMETRY_MAX_BYTES: usize = 2 * 1024 * 1024;
const TELEMETRY_SCAN_LINE_LIMIT: usize = 256;

#[derive(Debug, Clone)]
pub struct EdrTelemetryAlert {
    pub risk_bonus: u8,
    pub tag: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
struct EdrTelemetryCache {
    checked_at_ms: u64,
    alert: Option<EdrTelemetryAlert>,
}

pub struct EdrTelemetryGuard {
    telemetry_path: Option<PathBuf>,
    max_age_ms: u64,
    risk_bonus: u8,
    high_risk_tags: HashSet<String>,
    cache: Mutex<EdrTelemetryCache>,
}

impl EdrTelemetryGuard {
    pub fn new(cfg: &Config) -> Self {
        let high_risk_tags = cfg
            .security
            .edr_high_risk_tags
            .iter()
            .map(|value| normalize(value))
            .filter(|value| !value.is_empty())
            .collect::<HashSet<_>>();
        Self {
            telemetry_path: cfg.security.edr_telemetry_path.clone(),
            max_age_ms: cfg.security.edr_telemetry_max_age_secs.max(1) * 1_000,
            risk_bonus: cfg.security.edr_telemetry_risk_bonus.max(1),
            high_risk_tags,
            cache: Mutex::new(EdrTelemetryCache {
                checked_at_ms: 0,
                alert: None,
            }),
        }
    }

    pub async fn recent_alert(&self) -> Result<Option<EdrTelemetryAlert>> {
        let Some(path) = self.telemetry_path.as_ref() else {
            return Ok(None);
        };
        let now = now_ms();

        {
            let cache = self.cache.lock().await;
            if now.saturating_sub(cache.checked_at_ms) < TELEMETRY_CACHE_WINDOW_MS {
                return Ok(cache.alert.clone());
            }
        }

        let content = match tokio::fs::read_to_string(path).await {
            Ok(content) => content,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let mut cache = self.cache.lock().await;
                cache.checked_at_ms = now;
                cache.alert = None;
                return Ok(None);
            }
            Err(err) => {
                return Err(err).with_context(|| format!("read telemetry feed {}", path.display()));
            }
        };

        let clipped = clip_tail(&content, TELEMETRY_MAX_BYTES);
        let parsed = parse_latest_alert(
            clipped.as_str(),
            now,
            self.max_age_ms,
            self.risk_bonus,
            &self.high_risk_tags,
        );

        let mut cache = self.cache.lock().await;
        cache.checked_at_ms = now;
        cache.alert = parsed.clone();
        Ok(parsed)
    }
}

fn parse_latest_alert(
    input: &str,
    now_ms: u64,
    max_age_ms: u64,
    risk_bonus: u8,
    high_risk_tags: &HashSet<String>,
) -> Option<EdrTelemetryAlert> {
    for (scanned, line) in input.lines().rev().enumerate() {
        if scanned >= TELEMETRY_SCAN_LINE_LIMIT {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if let Some(alert) =
            classify_telemetry_value(value, now_ms, max_age_ms, risk_bonus, high_risk_tags)
        {
            return Some(alert);
        }
    }
    None
}

fn classify_telemetry_value(
    value: Value,
    now_ms: u64,
    max_age_ms: u64,
    risk_bonus: u8,
    high_risk_tags: &HashSet<String>,
) -> Option<EdrTelemetryAlert> {
    let timestamp_ms = extract_timestamp_ms(&value).unwrap_or(now_ms);
    if now_ms.saturating_sub(timestamp_ms) > max_age_ms {
        return None;
    }
    let severity = extract_string(&value, &["severity", "level"]);
    let tags = extract_tags(&value);
    let high_tag = tags
        .iter()
        .find(|tag| high_risk_tags.contains(*tag))
        .cloned();
    let high_severity = matches!(
        severity.as_deref(),
        Some("high" | "critical" | "severe" | "emergency")
    );
    let blocked_flag = extract_bool(&value, &["blocked", "quarantined"]).unwrap_or(false);
    if !(high_severity || high_tag.is_some() || blocked_flag) {
        return None;
    }

    let reason = if let Some(tag) = high_tag {
        format!("edr telemetry high-risk tag detected: {tag}")
    } else if high_severity {
        format!(
            "edr telemetry severity detected: {}",
            severity.unwrap_or_else(|| "unknown".to_owned())
        )
    } else {
        "edr telemetry event indicates blocked/quarantined host activity".to_owned()
    };
    Some(EdrTelemetryAlert {
        risk_bonus,
        tag: "edr_telemetry_alert".to_owned(),
        reason,
    })
}

fn extract_timestamp_ms(value: &Value) -> Option<u64> {
    for key in [
        "observedAtMs",
        "observed_at_ms",
        "timestampMs",
        "timestamp_ms",
        "ts",
    ] {
        if let Some(number) = value.get(key).and_then(Value::as_u64) {
            return Some(number);
        }
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            if let Ok(number) = text.trim().parse::<u64>() {
                return Some(number);
            }
        }
    }
    None
}

fn extract_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .map(normalize)
            .filter(|raw| !raw.is_empty())
    })
}

fn extract_bool(value: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_bool))
}

fn extract_tags(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(items) = value.get("tags").and_then(Value::as_array) {
        for item in items {
            if let Some(text) = item.as_str() {
                let normalized = normalize(text);
                if !normalized.is_empty() {
                    out.push(normalized);
                }
            }
        }
    }
    out
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(' ', "_")
}

fn clip_tail(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut start = value.len().saturating_sub(max_bytes);
    while start < value.len() && !value.is_char_boundary(start) {
        start = start.saturating_add(1);
    }
    value[start..].to_owned()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::config::Config;

    use super::EdrTelemetryGuard;

    static FEED_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn unique_feed_path(test_name: &str) -> PathBuf {
        let mut feed = std::env::temp_dir();
        let sequence = FEED_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        feed.push(format!(
            "openclaw-rs-edr-feed-{test_name}-{}-{}-{}.jsonl",
            std::process::id(),
            super::now_ms(),
            sequence
        ));
        feed
    }

    #[tokio::test]
    async fn telemetry_alerts_on_recent_high_severity_event() {
        let mut cfg = Config::default();
        let feed = unique_feed_path("high-severity");
        let now = super::now_ms();
        let payload = format!(
            "{{\"timestampMs\":{},\"severity\":\"critical\",\"tags\":[\"benign\"]}}\n",
            now
        );
        tokio::fs::write(&feed, payload).await.expect("write feed");

        cfg.security.edr_telemetry_path = Some(feed.clone());
        cfg.security.edr_telemetry_max_age_secs = 300;
        cfg.security.edr_telemetry_risk_bonus = 50;
        let guard = EdrTelemetryGuard::new(&cfg);
        let alert = guard
            .recent_alert()
            .await
            .expect("telemetry read")
            .expect("telemetry alert");
        assert_eq!(alert.tag, "edr_telemetry_alert");
        assert_eq!(alert.risk_bonus, 50);

        let _ = tokio::fs::remove_file(PathBuf::from(&feed)).await;
    }

    #[tokio::test]
    async fn telemetry_ignores_stale_events() {
        let mut cfg = Config::default();
        let feed = unique_feed_path("stale");
        let stale = super::now_ms().saturating_sub(600_000);
        let payload = format!(
            "{{\"timestampMs\":{},\"severity\":\"critical\",\"tags\":[\"ransomware\"]}}\n",
            stale
        );
        tokio::fs::write(&feed, payload).await.expect("write feed");

        cfg.security.edr_telemetry_path = Some(feed.clone());
        cfg.security.edr_telemetry_max_age_secs = 30;
        let guard = EdrTelemetryGuard::new(&cfg);
        let alert = guard.recent_alert().await.expect("telemetry read");
        assert!(alert.is_none());

        let _ = tokio::fs::remove_file(PathBuf::from(&feed)).await;
    }
}
