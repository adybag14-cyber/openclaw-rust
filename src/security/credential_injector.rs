use std::collections::HashSet;

use regex::Regex;
use serde_json::Value;

use crate::config::{ToolRuntimeCredentialPolicyConfig, ToolRuntimeLeakAction};

#[derive(Debug, Clone)]
pub struct CredentialInjection {
    pub env_pairs: Vec<(String, String)>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct LeakScan {
    pub leaked: bool,
    pub matches: usize,
    pub redacted: String,
    pub action: ToolRuntimeLeakAction,
}

#[derive(Debug, Clone)]
pub struct CredentialInjector {
    enabled: bool,
    env_allowlist: HashSet<String>,
    patterns: Vec<Regex>,
    leak_action: ToolRuntimeLeakAction,
    redaction_token: String,
}

impl CredentialInjector {
    pub fn new(cfg: ToolRuntimeCredentialPolicyConfig) -> anyhow::Result<Self> {
        let mut patterns = Vec::new();
        for raw in cfg.leak_patterns {
            let pattern = raw.trim();
            if pattern.is_empty() {
                continue;
            }
            patterns.push(Regex::new(pattern).map_err(|err| {
                anyhow::anyhow!("invalid leak detection regex `{pattern}`: {err}")
            })?);
        }

        Ok(Self {
            enabled: cfg.enabled,
            env_allowlist: cfg
                .env_allowlist
                .into_iter()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .collect(),
            patterns,
            leak_action: cfg.leak_action,
            redaction_token: cfg.redaction_token,
        })
    }

    pub fn inject_env_from_args(&self, args: &Value) -> CredentialInjection {
        if !self.enabled {
            return CredentialInjection {
                env_pairs: Vec::new(),
                warnings: Vec::new(),
            };
        }

        let requested = parse_inject_env_names(args);
        if requested.is_empty() {
            return CredentialInjection {
                env_pairs: Vec::new(),
                warnings: Vec::new(),
            };
        }

        let mut warnings = Vec::new();
        let mut env_pairs = Vec::new();

        for key in requested {
            if !self.env_allowlist.contains(&key) {
                warnings.push(format!(
                    "credential injection denied for env `{key}` (not allowlisted)"
                ));
                continue;
            }
            match std::env::var(&key) {
                Ok(value) => env_pairs.push((key, value)),
                Err(_) => warnings.push(format!(
                    "credential injection requested missing env `{key}`"
                )),
            }
        }

        CredentialInjection {
            env_pairs,
            warnings,
        }
    }

    pub fn scan_text(&self, text: &str) -> LeakScan {
        if !self.enabled || self.patterns.is_empty() {
            return LeakScan {
                leaked: false,
                matches: 0,
                redacted: text.to_owned(),
                action: self.leak_action,
            };
        }

        let mut total_matches = 0usize;
        let mut redacted = text.to_owned();
        for regex in &self.patterns {
            total_matches += regex.find_iter(&redacted).count();
            redacted = regex
                .replace_all(&redacted, self.redaction_token.as_str())
                .to_string();
        }

        LeakScan {
            leaked: total_matches > 0,
            matches: total_matches,
            redacted,
            action: self.leak_action,
        }
    }
}

fn parse_inject_env_names(args: &Value) -> Vec<String> {
    let mut out = Vec::new();

    if let Some(raw) = args.get("injectEnv") {
        match raw {
            Value::String(single) => push_key(&mut out, single),
            Value::Array(items) => {
                for item in items {
                    if let Some(raw) = item.as_str() {
                        push_key(&mut out, raw);
                    }
                }
            }
            _ => {}
        }
    }

    if let Some(raw) = args.get("inject_env") {
        if let Some(single) = raw.as_str() {
            push_key(&mut out, single);
        }
    }

    out
}

fn push_key(target: &mut Vec<String>, raw: &str) {
    let normalized = raw.trim().to_owned();
    if !normalized.is_empty() && !target.contains(&normalized) {
        target.push(normalized);
    }
}
