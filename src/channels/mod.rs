use std::time::Duration;

use serde_json::Value;

use crate::types::ActionRequest;

pub const DEFAULT_TEXT_CHUNK_LIMIT: usize = 4_000;
pub const DISCORD_TEXT_CHUNK_LIMIT: usize = 2_000;
pub const WAVE1_CHANNEL_ORDER: &[&str] = &[
    "telegram", "whatsapp", "discord", "slack", "signal", "webchat",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatType {
    Direct,
    Group,
    Channel,
}

impl ChatType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Group => "group",
            Self::Channel => "channel",
        }
    }
}

pub fn normalize_chat_type(raw: Option<&str>) -> Option<ChatType> {
    let value = raw?.trim().to_ascii_lowercase();
    match value.as_str() {
        "direct" | "dm" => Some(ChatType::Direct),
        "group" => Some(ChatType::Group),
        "channel" => Some(ChatType::Channel),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkMode {
    Length,
    Newline,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetryBackoffPolicy {
    pub initial_ms: u64,
    pub max_ms: u64,
    pub factor: f64,
    pub jitter: f64,
}

impl RetryBackoffPolicy {
    pub const fn bridge_default() -> Self {
        Self {
            initial_ms: 1_000,
            max_ms: 30_000,
            factor: 2.0,
            jitter: 0.0,
        }
    }

    fn sanitized(self) -> Self {
        let initial_ms = self.initial_ms.max(250);
        let max_ms = self.max_ms.max(initial_ms);
        let factor = self.factor.clamp(1.1, 10.0);
        let jitter = self.jitter.clamp(0.0, 1.0);
        Self {
            initial_ms,
            max_ms,
            factor,
            jitter,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MentionGateParams {
    pub require_mention: bool,
    pub can_detect_mention: bool,
    pub was_mentioned: bool,
    pub implicit_mention: bool,
    pub should_bypass_mention: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MentionGateResult {
    pub effective_was_mentioned: bool,
    pub should_skip: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MentionGateWithBypassParams {
    pub is_group: bool,
    pub require_mention: bool,
    pub can_detect_mention: bool,
    pub was_mentioned: bool,
    pub implicit_mention: bool,
    pub has_any_mention: bool,
    pub allow_text_commands: bool,
    pub has_control_command: bool,
    pub command_authorized: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MentionGateWithBypassResult {
    pub effective_was_mentioned: bool,
    pub should_skip: bool,
    pub should_bypass_mention: bool,
}

pub fn resolve_mention_gating(params: MentionGateParams) -> MentionGateResult {
    let effective_was_mentioned =
        params.was_mentioned || params.implicit_mention || params.should_bypass_mention;
    let should_skip =
        params.require_mention && params.can_detect_mention && !effective_was_mentioned;
    MentionGateResult {
        effective_was_mentioned,
        should_skip,
    }
}

pub fn resolve_mention_gating_with_bypass(
    params: MentionGateWithBypassParams,
) -> MentionGateWithBypassResult {
    let should_bypass_mention = params.is_group
        && params.require_mention
        && !params.was_mentioned
        && !params.has_any_mention
        && params.allow_text_commands
        && params.command_authorized
        && params.has_control_command;
    let base = resolve_mention_gating(MentionGateParams {
        require_mention: params.require_mention,
        can_detect_mention: params.can_detect_mention,
        was_mentioned: params.was_mentioned,
        implicit_mention: params.implicit_mention,
        should_bypass_mention,
    });
    MentionGateWithBypassResult {
        effective_was_mentioned: base.effective_was_mentioned,
        should_skip: base.should_skip,
        should_bypass_mention,
    }
}

pub fn normalize_channel_id(raw: Option<&str>) -> Option<String> {
    let value = raw?.trim().to_ascii_lowercase();
    if value.is_empty() {
        return None;
    }
    let normalized = match value.as_str() {
        "tg" | "grammy" => "telegram",
        "wa" | "baileys" => "whatsapp",
        "signal-cli" => "signal",
        "web-chat" | "web_chat" | "webchat-ui" => "webchat",
        _ => value.as_str(),
    };
    Some(normalized.to_owned())
}

pub fn default_text_chunk_limit(channel: Option<&str>) -> usize {
    match normalize_channel_id(channel).as_deref() {
        Some("discord") => DISCORD_TEXT_CHUNK_LIMIT,
        _ => DEFAULT_TEXT_CHUNK_LIMIT,
    }
}

pub fn default_chunk_mode(channel: Option<&str>) -> ChunkMode {
    match normalize_channel_id(channel).as_deref() {
        Some("webchat") => ChunkMode::Newline,
        _ => ChunkMode::Length,
    }
}

pub fn chunk_text_with_mode(text: &str, limit: usize, mode: ChunkMode) -> Vec<String> {
    match mode {
        ChunkMode::Length => chunk_by_length(text, limit),
        ChunkMode::Newline => chunk_by_paragraph(text, limit),
    }
}

pub fn compute_retry_backoff_delay_ms(policy: RetryBackoffPolicy, attempt: u32) -> u64 {
    let policy = policy.sanitized();
    let effective_attempt = attempt.max(1);
    let mut base = policy.initial_ms as f64;
    for _ in 1..effective_attempt {
        base = (base * policy.factor).min(policy.max_ms as f64);
    }
    let jitter = base * policy.jitter * deterministic_unit_interval(effective_attempt);
    let delay = (base + jitter).round();
    let bounded = delay.clamp(policy.initial_ms as f64, policy.max_ms as f64);
    bounded as u64
}

pub fn compute_retry_backoff_delay(policy: RetryBackoffPolicy, attempt: u32) -> Duration {
    Duration::from_millis(compute_retry_backoff_delay_ms(policy, attempt))
}

fn deterministic_unit_interval(attempt: u32) -> f64 {
    let mut x = u64::from(attempt).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    (x & 0xFFFF) as f64 / 65_535.0
}

fn chunk_by_length(text: &str, limit: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    if limit == 0 {
        return vec![text.to_owned()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text.trim();

    while !remaining.is_empty() {
        if remaining.chars().count() <= limit {
            chunks.push(remaining.to_owned());
            break;
        }

        let hard_boundary = nth_char_boundary(remaining, limit);
        let window = &remaining[..hard_boundary];
        let mut break_at = find_last_newline(window)
            .or_else(|| find_last_whitespace(window))
            .unwrap_or(hard_boundary);
        if break_at == 0 {
            break_at = hard_boundary;
        }

        let chunk = remaining[..break_at].trim_end();
        if !chunk.is_empty() {
            chunks.push(chunk.to_owned());
        }

        let mut next = &remaining[break_at..];
        if let Some(first) = next.chars().next() {
            if first.is_whitespace() {
                next = &next[first.len_utf8()..];
            }
        }
        remaining = next.trim_start();
    }

    chunks
}

fn chunk_by_paragraph(text: &str, limit: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    if limit == 0 {
        return vec![text.to_owned()];
    }
    let paragraphs = split_paragraphs(text);
    if paragraphs.is_empty() {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    for paragraph in paragraphs {
        if paragraph.chars().count() <= limit {
            chunks.push(paragraph);
        } else {
            chunks.extend(chunk_by_length(&paragraph, limit));
        }
    }
    chunks
}

fn split_paragraphs(text: &str) -> Vec<String> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut paragraphs = Vec::new();
    let mut current = Vec::new();
    let mut blank_seen = false;

    for line in normalized.split('\n') {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(current.join("\n"));
                current.clear();
            }
            blank_seen = true;
            continue;
        }
        if blank_seen && !current.is_empty() {
            paragraphs.push(current.join("\n"));
            current.clear();
        }
        blank_seen = false;
        current.push(line.to_owned());
    }

    if !current.is_empty() {
        paragraphs.push(current.join("\n"));
    }

    paragraphs
}

fn nth_char_boundary(value: &str, nth: usize) -> usize {
    if nth == 0 {
        return 0;
    }
    match value.char_indices().nth(nth) {
        Some((idx, _)) => idx,
        None => value.len(),
    }
}

fn find_last_newline(value: &str) -> Option<usize> {
    value
        .char_indices()
        .rev()
        .find_map(|(idx, ch)| (ch == '\n').then_some(idx))
}

fn find_last_whitespace(value: &str) -> Option<usize> {
    value
        .char_indices()
        .rev()
        .find_map(|(idx, ch)| (ch.is_whitespace()).then_some(idx))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelCapabilities {
    pub name: &'static str,
    pub supports_edit: bool,
    pub supports_delete: bool,
    pub supports_reactions: bool,
    pub supports_threads: bool,
    pub supports_polls: bool,
    pub supports_media: bool,
    pub default_dm_pairing: bool,
}

pub trait ChannelDriver: Send + Sync {
    fn extract(&self, frame: &Value) -> Option<ActionRequest>;
    fn capabilities(&self) -> ChannelCapabilities;
}

pub struct DriverRegistry {
    drivers: Vec<Box<dyn ChannelDriver>>,
}

impl DriverRegistry {
    pub fn default_registry() -> Self {
        let mut drivers: Vec<Box<dyn ChannelDriver>> = WAVE1_CHANNEL_ORDER
            .iter()
            .filter_map(|channel| driver_for_channel(channel))
            .collect();
        drivers.push(Box::new(GenericDriver));
        Self { drivers }
    }

    pub fn extract(&self, frame: &Value) -> Option<ActionRequest> {
        for driver in &self.drivers {
            if let Some(request) = driver.extract(frame) {
                return Some(request);
            }
        }
        None
    }

    pub fn capabilities(&self) -> Vec<ChannelCapabilities> {
        self.drivers.iter().map(|d| d.capabilities()).collect()
    }
}

fn driver_for_channel(channel: &str) -> Option<Box<dyn ChannelDriver>> {
    match channel {
        "telegram" => Some(Box::new(TelegramDriver)),
        "whatsapp" => Some(Box::new(WhatsAppDriver)),
        "discord" => Some(Box::new(DiscordDriver)),
        "slack" => Some(Box::new(SlackDriver)),
        "signal" => Some(Box::new(SignalDriver)),
        "webchat" => Some(Box::new(WebChatDriver)),
        _ => None,
    }
}

struct GenericDriver;

impl ChannelDriver for GenericDriver {
    fn extract(&self, frame: &Value) -> Option<ActionRequest> {
        ActionRequest::from_gateway_frame(frame)
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            name: "generic",
            supports_edit: false,
            supports_delete: false,
            supports_reactions: false,
            supports_threads: false,
            supports_polls: false,
            supports_media: true,
            default_dm_pairing: true,
        }
    }
}

struct DiscordDriver;

impl ChannelDriver for DiscordDriver {
    fn extract(&self, frame: &Value) -> Option<ActionRequest> {
        extract_with_hints(frame, "discord", &["discord"])
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            name: "discord",
            supports_edit: true,
            supports_delete: true,
            supports_reactions: true,
            supports_threads: true,
            supports_polls: true,
            supports_media: true,
            default_dm_pairing: true,
        }
    }
}

struct TelegramDriver;

impl ChannelDriver for TelegramDriver {
    fn extract(&self, frame: &Value) -> Option<ActionRequest> {
        extract_with_hints(frame, "telegram", &["telegram", "grammy", "tg"])
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            name: "telegram",
            supports_edit: true,
            supports_delete: true,
            supports_reactions: true,
            supports_threads: false,
            supports_polls: true,
            supports_media: true,
            default_dm_pairing: true,
        }
    }
}

struct SlackDriver;

impl ChannelDriver for SlackDriver {
    fn extract(&self, frame: &Value) -> Option<ActionRequest> {
        extract_with_hints(frame, "slack", &["slack"])
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            name: "slack",
            supports_edit: true,
            supports_delete: true,
            supports_reactions: true,
            supports_threads: true,
            supports_polls: false,
            supports_media: true,
            default_dm_pairing: true,
        }
    }
}

struct WhatsAppDriver;

impl ChannelDriver for WhatsAppDriver {
    fn extract(&self, frame: &Value) -> Option<ActionRequest> {
        extract_with_hints(frame, "whatsapp", &["whatsapp", "baileys", "wa"])
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            name: "whatsapp",
            supports_edit: false,
            supports_delete: false,
            supports_reactions: true,
            supports_threads: false,
            supports_polls: true,
            supports_media: true,
            default_dm_pairing: true,
        }
    }
}

struct SignalDriver;

impl ChannelDriver for SignalDriver {
    fn extract(&self, frame: &Value) -> Option<ActionRequest> {
        extract_with_hints(frame, "signal", &["signal", "signal-cli"])
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            name: "signal",
            supports_edit: false,
            supports_delete: false,
            supports_reactions: true,
            supports_threads: false,
            supports_polls: false,
            supports_media: true,
            default_dm_pairing: true,
        }
    }
}

struct WebChatDriver;

impl ChannelDriver for WebChatDriver {
    fn extract(&self, frame: &Value) -> Option<ActionRequest> {
        extract_with_hints(frame, "webchat", &["webchat", "web-chat", "web_chat"])
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            name: "webchat",
            supports_edit: true,
            supports_delete: true,
            supports_reactions: false,
            supports_threads: false,
            supports_polls: false,
            supports_media: true,
            default_dm_pairing: false,
        }
    }
}

fn normalize(input: &str) -> String {
    input.trim().to_ascii_lowercase()
}

fn extract_with_hints(
    frame: &Value,
    canonical_channel: &str,
    hints: &[&str],
) -> Option<ActionRequest> {
    let mut request = ActionRequest::from_gateway_frame(frame)?;
    if let Some(channel) = request.channel.as_deref() {
        let normalized = normalize_channel_id(Some(channel))?;
        if normalized == canonical_channel {
            request.channel = Some(canonical_channel.to_owned());
            return Some(request);
        }
        return None;
    }

    let source = frame
        .get("event")
        .and_then(Value::as_str)
        .or_else(|| frame.get("method").and_then(Value::as_str))
        .map(normalize);

    let matched = source
        .as_deref()
        .is_some_and(|src| hints.iter().any(|hint| src.contains(hint)));

    if matched {
        request.channel = Some(canonical_channel.to_owned());
        return Some(request);
    }

    None
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::{
        chunk_text_with_mode, compute_retry_backoff_delay, compute_retry_backoff_delay_ms,
        default_chunk_mode, default_text_chunk_limit, normalize_chat_type, resolve_mention_gating,
        resolve_mention_gating_with_bypass, ChatType, ChunkMode, DriverRegistry, MentionGateParams,
        MentionGateWithBypassParams, RetryBackoffPolicy, WAVE1_CHANNEL_ORDER,
    };

    #[test]
    fn discord_driver_sets_channel_from_event_name() {
        let registry = DriverRegistry::default_registry();
        let frame = json!({
            "type": "event",
            "event": "discord.message",
            "payload": {
                "id": "req-1",
                "command": "git status",
                "tool": "exec"
            }
        });
        let request = registry.extract(&frame).expect("request");
        assert_eq!(request.channel.as_deref(), Some("discord"));
    }

    #[test]
    fn signal_driver_detects_source() {
        let registry = DriverRegistry::default_registry();
        let frame = json!({
            "type": "event",
            "event": "signal.message",
            "payload": {
                "id": "req-signal",
                "tool": "exec",
                "command": "git status"
            }
        });
        let request = registry.extract(&frame).expect("request");
        assert_eq!(request.channel.as_deref(), Some("signal"));
    }

    #[test]
    fn webchat_driver_detects_source() {
        let registry = DriverRegistry::default_registry();
        let frame = json!({
            "type": "event",
            "event": "webchat.message",
            "payload": {
                "id": "req-webchat",
                "tool": "exec",
                "command": "git status"
            }
        });
        let request = registry.extract(&frame).expect("request");
        assert_eq!(request.channel.as_deref(), Some("webchat"));
    }

    #[test]
    fn generic_driver_fallback_still_extracts() {
        let registry = DriverRegistry::default_registry();
        let frame = json!({
            "type": "event",
            "event": "agent",
            "payload": {
                "id": "req-2",
                "tool": "exec",
                "command": "git status"
            }
        });
        let request = registry.extract(&frame).expect("request");
        assert_eq!(request.id, "req-2");
    }

    #[test]
    fn telegram_driver_detects_source() {
        let registry = DriverRegistry::default_registry();
        let frame = json!({
            "type": "event",
            "event": "telegram.message",
            "payload": {
                "id": "req-3",
                "tool": "exec",
                "command": "git status"
            }
        });
        let request = registry.extract(&frame).expect("request");
        assert_eq!(request.channel.as_deref(), Some("telegram"));
    }

    #[test]
    fn slack_driver_detects_source() {
        let registry = DriverRegistry::default_registry();
        let frame = json!({
            "type": "event",
            "event": "slack.message",
            "payload": {
                "id": "req-4",
                "tool": "exec",
                "command": "git status"
            }
        });
        let request = registry.extract(&frame).expect("request");
        assert_eq!(request.channel.as_deref(), Some("slack"));
    }

    #[test]
    fn whatsapp_driver_detects_source() {
        let registry = DriverRegistry::default_registry();
        let frame = json!({
            "type": "event",
            "event": "whatsapp.message",
            "payload": {
                "id": "req-5",
                "tool": "exec",
                "command": "git status"
            }
        });
        let request = registry.extract(&frame).expect("request");
        assert_eq!(request.channel.as_deref(), Some("whatsapp"));
    }

    #[test]
    fn exposes_channel_capabilities_and_wave1_order() {
        let registry = DriverRegistry::default_registry();
        let caps = registry.capabilities();
        let names = caps.iter().map(|cap| cap.name).collect::<Vec<_>>();

        for channel in WAVE1_CHANNEL_ORDER {
            assert!(names.contains(channel), "missing wave1 channel: {channel}");
        }
        assert!(caps
            .iter()
            .any(|c| c.name == "discord" && c.supports_threads));
        assert!(caps
            .iter()
            .any(|c| c.name == "signal" && c.supports_reactions));
        assert!(caps.iter().any(|c| c.name == "webchat" && c.supports_edit));
    }

    #[test]
    fn normalize_chat_type_supports_dm_alias() {
        assert_eq!(normalize_chat_type(Some("dm")), Some(ChatType::Direct));
        assert_eq!(normalize_chat_type(Some("direct")), Some(ChatType::Direct));
        assert_eq!(normalize_chat_type(Some("group")), Some(ChatType::Group));
        assert_eq!(
            normalize_chat_type(Some("channel")),
            Some(ChatType::Channel)
        );
        assert_eq!(normalize_chat_type(Some("unknown")), None);
    }

    #[test]
    fn mention_gate_skips_when_required_and_not_mentioned() {
        let result = resolve_mention_gating(MentionGateParams {
            require_mention: true,
            can_detect_mention: true,
            was_mentioned: false,
            implicit_mention: false,
            should_bypass_mention: false,
        });
        assert!(!result.effective_was_mentioned);
        assert!(result.should_skip);
    }

    #[test]
    fn mention_gate_with_bypass_allows_authorized_control_commands() {
        let result = resolve_mention_gating_with_bypass(MentionGateWithBypassParams {
            is_group: true,
            require_mention: true,
            can_detect_mention: true,
            was_mentioned: false,
            implicit_mention: false,
            has_any_mention: false,
            allow_text_commands: true,
            has_control_command: true,
            command_authorized: true,
        });
        assert!(result.should_bypass_mention);
        assert!(result.effective_was_mentioned);
        assert!(!result.should_skip);
    }

    #[test]
    fn chunking_supports_length_and_newline_modes() {
        let text = "one two three four five six";
        let length_chunks = chunk_text_with_mode(text, 10, ChunkMode::Length);
        assert!(length_chunks.len() >= 3);
        assert!(length_chunks
            .iter()
            .all(|chunk| chunk.chars().count() <= 10));

        let newline_text = "para one line\nstill one\n\npara two";
        let newline_chunks = chunk_text_with_mode(newline_text, 100, ChunkMode::Newline);
        assert_eq!(newline_chunks, vec!["para one line\nstill one", "para two"]);
    }

    #[test]
    fn default_chunk_limit_matches_core_channel_defaults() {
        assert_eq!(default_text_chunk_limit(Some("discord")), 2_000);
        assert_eq!(default_text_chunk_limit(Some("telegram")), 4_000);
        assert_eq!(default_text_chunk_limit(Some("signal")), 4_000);
        assert_eq!(default_text_chunk_limit(Some("webchat")), 4_000);
        assert_eq!(default_chunk_mode(Some("webchat")), ChunkMode::Newline);
        assert_eq!(default_chunk_mode(Some("discord")), ChunkMode::Length);
    }

    #[test]
    fn retry_backoff_policy_scales_and_caps() {
        let policy = RetryBackoffPolicy {
            initial_ms: 1_000,
            max_ms: 30_000,
            factor: 2.0,
            jitter: 0.0,
        };
        assert_eq!(compute_retry_backoff_delay_ms(policy, 1), 1_000);
        assert_eq!(compute_retry_backoff_delay_ms(policy, 2), 2_000);
        assert_eq!(compute_retry_backoff_delay_ms(policy, 3), 4_000);
        assert_eq!(compute_retry_backoff_delay_ms(policy, 8), 30_000);
        assert_eq!(
            compute_retry_backoff_delay(policy, 2),
            Duration::from_millis(2_000)
        );
    }

    #[test]
    fn chat_type_as_str_matches_normalized_values() {
        assert_eq!(ChatType::Direct.as_str(), "direct");
        assert_eq!(ChatType::Group.as_str(), "group");
        assert_eq!(ChatType::Channel.as_str(), "channel");
    }
}
