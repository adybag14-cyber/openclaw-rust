use std::collections::HashMap;

use crate::config::{ToolRuntimePolicyConfig, ToolRuntimePolicyRule};

const GROUP_MEMORY: &[&str] = &["memory_search", "memory_get"];
const GROUP_WEB: &[&str] = &["web_search", "web_fetch"];
const GROUP_FS: &[&str] = &["read", "write", "edit", "apply_patch"];
const GROUP_RUNTIME: &[&str] = &["exec", "process", "wasm"];
const GROUP_SESSIONS: &[&str] = &[
    "sessions",
    "sessions_list",
    "sessions_history",
    "sessions_send",
    "sessions_spawn",
    "subagents",
    "session_status",
];
const GROUP_UI: &[&str] = &["browser", "canvas"];
const GROUP_AUTOMATION: &[&str] = &["cron", "gateway", "routines"];
const GROUP_MESSAGING: &[&str] = &["message"];
const GROUP_NODES: &[&str] = &["nodes"];
const GROUP_OPENCLAW: &[&str] = &[
    "browser",
    "canvas",
    "nodes",
    "cron",
    "message",
    "gateway",
    "routines",
    "wasm",
    "agents_list",
    "sessions_list",
    "sessions_history",
    "sessions_send",
    "sessions_spawn",
    "subagents",
    "session_status",
    "memory_search",
    "memory_get",
    "web_search",
    "web_fetch",
    "image",
];

pub struct ToolPolicyMatcher {
    config: ToolRuntimePolicyConfig,
}

impl ToolPolicyMatcher {
    pub fn new(config: ToolRuntimePolicyConfig) -> Self {
        Self { config }
    }

    pub fn allows(
        &self,
        tool_name: &str,
        model_provider: Option<&str>,
        model_id: Option<&str>,
    ) -> bool {
        let normalized_tool = normalize_tool_name(tool_name);
        let provider_policy =
            resolve_provider_policy(&self.config.by_provider, model_provider, model_id);

        let mut steps: Vec<(Vec<String>, Vec<String>)> = Vec::new();
        if let Some(profile_step) = profile_policy(self.config.profile.as_deref()) {
            steps.push(profile_step);
        }
        if let Some(policy) = provider_policy {
            if let Some(profile_step) = profile_policy(policy.profile.as_deref()) {
                steps.push(profile_step);
            }
        }

        steps.push((self.config.allow.clone(), self.config.deny.clone()));
        if let Some(policy) = provider_policy {
            steps.push((policy.allow.clone(), policy.deny.clone()));
        }

        steps
            .iter()
            .all(|(allow, deny)| is_allowed_by_step(&normalized_tool, allow, deny))
    }
}

fn resolve_provider_policy<'a>(
    by_provider: &'a HashMap<String, ToolRuntimePolicyRule>,
    model_provider: Option<&str>,
    model_id: Option<&str>,
) -> Option<&'a ToolRuntimePolicyRule> {
    let provider = model_provider
        .map(normalize_entry)
        .filter(|value| !value.is_empty())?;
    let model = model_id
        .map(normalize_entry)
        .filter(|value| !value.is_empty());

    if let Some(model_id) = model {
        let full = if model_id.contains('/') {
            model_id
        } else {
            format!("{provider}/{model_id}")
        };
        if let Some(policy) = by_provider.get(&full) {
            return Some(policy);
        }
    }

    by_provider.get(&provider)
}

fn profile_policy(profile: Option<&str>) -> Option<(Vec<String>, Vec<String>)> {
    let normalized = profile.map(normalize_entry)?;
    match normalized.as_str() {
        "minimal" => Some((vec!["session_status".to_owned()], Vec::new())),
        "coding" => Some((
            vec![
                "group:fs".to_owned(),
                "group:runtime".to_owned(),
                "group:sessions".to_owned(),
                "group:memory".to_owned(),
                "image".to_owned(),
            ],
            Vec::new(),
        )),
        "messaging" => Some((
            vec![
                "group:messaging".to_owned(),
                "sessions_list".to_owned(),
                "sessions_history".to_owned(),
                "sessions_send".to_owned(),
                "session_status".to_owned(),
            ],
            Vec::new(),
        )),
        "full" => Some((Vec::new(), Vec::new())),
        _ => None,
    }
}

fn is_allowed_by_step(tool_name: &str, allow: &[String], deny: &[String]) -> bool {
    let expanded_allow = expand_entries(allow);
    let expanded_deny = expand_entries(deny);

    if expanded_deny
        .iter()
        .any(|pattern| wildcard_match(pattern, tool_name))
    {
        return false;
    }

    if expanded_allow.is_empty() {
        return true;
    }

    if expanded_allow
        .iter()
        .any(|pattern| wildcard_match(pattern, tool_name))
    {
        return true;
    }

    if tool_name == "apply_patch"
        && expanded_allow
            .iter()
            .any(|pattern| wildcard_match(pattern, "exec"))
    {
        return true;
    }

    false
}

fn expand_entries(entries: &[String]) -> Vec<String> {
    let mut expanded = Vec::new();
    for entry in entries {
        let normalized = normalize_entry(entry);
        match normalized.as_str() {
            "group:memory" => expanded.extend(GROUP_MEMORY.iter().map(|value| (*value).to_owned())),
            "group:web" => expanded.extend(GROUP_WEB.iter().map(|value| (*value).to_owned())),
            "group:fs" => expanded.extend(GROUP_FS.iter().map(|value| (*value).to_owned())),
            "group:runtime" => {
                expanded.extend(GROUP_RUNTIME.iter().map(|value| (*value).to_owned()))
            }
            "group:sessions" => {
                expanded.extend(GROUP_SESSIONS.iter().map(|value| (*value).to_owned()))
            }
            "group:ui" => expanded.extend(GROUP_UI.iter().map(|value| (*value).to_owned())),
            "group:automation" => {
                expanded.extend(GROUP_AUTOMATION.iter().map(|value| (*value).to_owned()))
            }
            "group:messaging" => {
                expanded.extend(GROUP_MESSAGING.iter().map(|value| (*value).to_owned()))
            }
            "group:nodes" => expanded.extend(GROUP_NODES.iter().map(|value| (*value).to_owned())),
            "group:openclaw" => {
                expanded.extend(GROUP_OPENCLAW.iter().map(|value| (*value).to_owned()))
            }
            _ if !normalized.is_empty() => expanded.push(normalized),
            _ => {}
        }
    }
    expanded
}

fn normalize_tool_name(value: &str) -> String {
    let normalized = normalize_entry(value);
    match normalized.as_str() {
        "bash" => "exec".to_owned(),
        "apply-patch" => "apply_patch".to_owned(),
        "session" => "sessions".to_owned(),
        _ => normalized,
    }
}

fn normalize_entry(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == value;
    }

    let parts = pattern.split('*').collect::<Vec<_>>();
    let mut cursor = 0usize;

    if let Some(first) = parts.first() {
        if !first.is_empty() {
            if !value.starts_with(first) {
                return false;
            }
            cursor = first.len();
        }
    }

    let middle_start = if parts.first().is_some_and(|part| part.is_empty()) {
        0
    } else {
        1
    };
    let middle_end = if parts.last().is_some_and(|part| part.is_empty()) {
        parts.len()
    } else {
        parts.len().saturating_sub(1)
    };

    for part in parts.iter().take(middle_end).skip(middle_start) {
        if part.is_empty() {
            continue;
        }
        let Some(found) = value[cursor..].find(part) else {
            return false;
        };
        cursor += found + part.len();
    }

    if let Some(last) = parts.last() {
        if !last.is_empty() {
            return value[cursor..].ends_with(last);
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::config::{ToolRuntimePolicyConfig, ToolRuntimePolicyRule};

    use super::ToolPolicyMatcher;

    #[test]
    fn profile_coding_expands_group_runtime_and_fs() {
        let matcher = ToolPolicyMatcher::new(ToolRuntimePolicyConfig {
            profile: Some("coding".to_owned()),
            ..ToolRuntimePolicyConfig::default()
        });

        assert!(matcher.allows("exec", None, None));
        assert!(matcher.allows("read", None, None));
        assert!(matcher.allows("sessions", None, None));
        assert!(!matcher.allows("gateway", None, None));
    }

    #[test]
    fn deny_takes_precedence_over_allow() {
        let matcher = ToolPolicyMatcher::new(ToolRuntimePolicyConfig {
            allow: vec!["group:runtime".to_owned()],
            deny: vec!["exec".to_owned()],
            ..ToolRuntimePolicyConfig::default()
        });

        assert!(!matcher.allows("exec", None, None));
        assert!(matcher.allows("process", None, None));
    }

    #[test]
    fn provider_specific_rule_is_applied_after_global_policy() {
        let mut by_provider = HashMap::new();
        by_provider.insert(
            "openai".to_owned(),
            ToolRuntimePolicyRule {
                allow: vec!["group:fs".to_owned()],
                deny: vec!["write".to_owned()],
                ..ToolRuntimePolicyRule::default()
            },
        );
        let matcher = ToolPolicyMatcher::new(ToolRuntimePolicyConfig {
            allow: vec!["group:fs".to_owned()],
            by_provider,
            ..ToolRuntimePolicyConfig::default()
        });

        assert!(matcher.allows("read", Some("openai"), Some("gpt-5")));
        assert!(!matcher.allows("write", Some("openai"), Some("gpt-5")));
        assert!(matcher.allows("write", Some("anthropic"), Some("claude-4")));
    }

    #[test]
    fn provider_model_specific_rule_beats_provider_fallback() {
        let mut by_provider = HashMap::new();
        by_provider.insert(
            "openai".to_owned(),
            ToolRuntimePolicyRule {
                allow: vec!["group:runtime".to_owned()],
                ..ToolRuntimePolicyRule::default()
            },
        );
        by_provider.insert(
            "openai/gpt-5".to_owned(),
            ToolRuntimePolicyRule {
                allow: vec!["read".to_owned()],
                ..ToolRuntimePolicyRule::default()
            },
        );
        let matcher = ToolPolicyMatcher::new(ToolRuntimePolicyConfig {
            by_provider,
            ..ToolRuntimePolicyConfig::default()
        });

        assert!(matcher.allows("read", Some("openai"), Some("gpt-5")));
        assert!(!matcher.allows("exec", Some("openai"), Some("gpt-5")));
        assert!(matcher.allows("exec", Some("openai"), Some("gpt-4o")));
    }

    #[test]
    fn allowlisted_exec_implies_apply_patch() {
        let matcher = ToolPolicyMatcher::new(ToolRuntimePolicyConfig {
            allow: vec!["exec".to_owned()],
            ..ToolRuntimePolicyConfig::default()
        });

        assert!(matcher.allows("apply_patch", None, None));
    }
}
