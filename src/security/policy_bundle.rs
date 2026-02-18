use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::Sha256;

use crate::config::{Config, PolicyAction};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone)]
pub struct PolicyBundleLoadReport {
    pub path: PathBuf,
    pub version: u32,
    pub bundle_id: Option<String>,
    pub overridden_fields: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct UnsignedPolicyBundle {
    #[serde(default = "default_bundle_version")]
    version: u32,
    #[serde(rename = "bundleId", alias = "bundle_id")]
    bundle_id: Option<String>,
    #[serde(rename = "signedAt", alias = "signed_at")]
    signed_at: Option<String>,
    #[serde(default)]
    policy: PolicyBundlePatch,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PolicyBundlePatch {
    #[serde(rename = "reviewThreshold", alias = "review_threshold")]
    review_threshold: Option<u8>,
    #[serde(rename = "blockThreshold", alias = "block_threshold")]
    block_threshold: Option<u8>,
    #[serde(
        rename = "allowedCommandPrefixes",
        alias = "allowed_command_prefixes",
        default
    )]
    allowed_command_prefixes: Option<Vec<String>>,
    #[serde(
        rename = "blockedCommandPatterns",
        alias = "blocked_command_patterns",
        default
    )]
    blocked_command_patterns: Option<Vec<String>>,
    #[serde(
        rename = "promptInjectionPatterns",
        alias = "prompt_injection_patterns",
        default
    )]
    prompt_injection_patterns: Option<Vec<String>>,
    #[serde(rename = "toolPolicies", alias = "tool_policies", default)]
    tool_policies: Option<BTreeMap<String, PolicyAction>>,
    #[serde(rename = "toolRiskBonus", alias = "tool_risk_bonus", default)]
    tool_risk_bonus: Option<BTreeMap<String, u8>>,
    #[serde(rename = "channelRiskBonus", alias = "channel_risk_bonus", default)]
    channel_risk_bonus: Option<BTreeMap<String, u8>>,
}

fn default_bundle_version() -> u32 {
    1
}

pub async fn apply_signed_policy_bundle(
    cfg: &mut Config,
) -> Result<Option<PolicyBundleLoadReport>> {
    let Some(path) = cfg.security.policy_bundle_path.clone() else {
        return Ok(None);
    };
    if path.as_os_str().to_string_lossy().trim().is_empty() {
        return Ok(None);
    }
    let Some(bundle_key) = cfg.security.policy_bundle_key.clone() else {
        return Err(anyhow!(
            "policy bundle key missing; set security.policy_bundle_key or OPENCLAW_RS_POLICY_BUNDLE_KEY"
        ));
    };
    let bundle_key = bundle_key.trim().to_owned();
    if bundle_key.is_empty() {
        return Err(anyhow!("policy bundle key cannot be empty"));
    }

    let raw = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("failed reading policy bundle {}", path.display()))?;
    let root: Value = serde_json::from_str(&raw)
        .with_context(|| format!("failed parsing policy bundle JSON {}", path.display()))?;
    let mut root_obj = root
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow!("policy bundle root must be an object"))?;
    let signature_raw = root_obj
        .remove("signature")
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .ok_or_else(|| anyhow!("policy bundle signature missing"))?;

    verify_bundle_signature(&root_obj, &bundle_key, &signature_raw)?;

    let unsigned_value = Value::Object(root_obj);
    let bundle: UnsignedPolicyBundle = serde_json::from_value(unsigned_value)
        .with_context(|| format!("invalid policy bundle schema {}", path.display()))?;

    let mut overridden_fields = Vec::new();
    apply_bundle_patch(cfg, &bundle.policy, &mut overridden_fields)?;
    validate_thresholds(cfg)?;

    if let Some(value) = bundle.signed_at.as_deref() {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            overridden_fields.push("signed_at".to_owned());
        }
    }

    Ok(Some(PolicyBundleLoadReport {
        path,
        version: bundle.version,
        bundle_id: normalize_optional_text(bundle.bundle_id.as_deref()),
        overridden_fields,
    }))
}

fn apply_bundle_patch(
    cfg: &mut Config,
    patch: &PolicyBundlePatch,
    overridden_fields: &mut Vec<String>,
) -> Result<()> {
    if let Some(review_threshold) = patch.review_threshold {
        cfg.security.review_threshold = review_threshold;
        overridden_fields.push("review_threshold".to_owned());
    }
    if let Some(block_threshold) = patch.block_threshold {
        cfg.security.block_threshold = block_threshold;
        overridden_fields.push("block_threshold".to_owned());
    }
    if let Some(prefixes) = patch.allowed_command_prefixes.as_ref() {
        cfg.security.allowed_command_prefixes = normalize_string_list(prefixes);
        overridden_fields.push("allowed_command_prefixes".to_owned());
    }
    if let Some(patterns) = patch.blocked_command_patterns.as_ref() {
        cfg.security.blocked_command_patterns = normalize_string_list(patterns);
        overridden_fields.push("blocked_command_patterns".to_owned());
    }
    if let Some(patterns) = patch.prompt_injection_patterns.as_ref() {
        cfg.security.prompt_injection_patterns = normalize_string_list(patterns);
        overridden_fields.push("prompt_injection_patterns".to_owned());
    }
    if let Some(map) = patch.tool_policies.as_ref() {
        cfg.security.tool_policies = map
            .iter()
            .filter_map(|(key, value)| {
                let normalized = normalize_key(key);
                if normalized.is_empty() {
                    None
                } else {
                    Some((normalized, *value))
                }
            })
            .collect();
        overridden_fields.push("tool_policies".to_owned());
    }
    if let Some(map) = patch.tool_risk_bonus.as_ref() {
        cfg.security.tool_risk_bonus = map
            .iter()
            .filter_map(|(key, value)| {
                let normalized = normalize_key(key);
                if normalized.is_empty() {
                    None
                } else {
                    Some((normalized, *value))
                }
            })
            .collect();
        overridden_fields.push("tool_risk_bonus".to_owned());
    }
    if let Some(map) = patch.channel_risk_bonus.as_ref() {
        cfg.security.channel_risk_bonus = map
            .iter()
            .filter_map(|(key, value)| {
                let normalized = normalize_key(key);
                if normalized.is_empty() {
                    None
                } else {
                    Some((normalized, *value))
                }
            })
            .collect();
        overridden_fields.push("channel_risk_bonus".to_owned());
    }

    if patch.allowed_command_prefixes.is_some() && cfg.security.allowed_command_prefixes.is_empty()
    {
        return Err(anyhow!(
            "policy bundle applied empty allowed command prefixes; provide at least one prefix"
        ));
    }
    if patch.prompt_injection_patterns.is_some()
        && cfg.security.prompt_injection_patterns.is_empty()
    {
        return Err(anyhow!(
            "policy bundle applied empty prompt injection pattern list; provide at least one pattern"
        ));
    }
    if patch.blocked_command_patterns.is_some() && cfg.security.blocked_command_patterns.is_empty()
    {
        return Err(anyhow!(
            "policy bundle applied empty blocked command pattern list; provide at least one pattern"
        ));
    }

    Ok(())
}

fn validate_thresholds(cfg: &Config) -> Result<()> {
    if cfg.security.review_threshold >= cfg.security.block_threshold {
        return Err(anyhow!(
            "policy bundle produced invalid thresholds: review_threshold must be lower than block_threshold"
        ));
    }
    Ok(())
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

fn normalize_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_string_list(values: &[String]) -> Vec<String> {
    values
        .iter()
        .filter_map(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        })
        .collect()
}

fn verify_bundle_signature(
    unsigned_bundle: &Map<String, Value>,
    key: &str,
    provided_signature: &str,
) -> Result<()> {
    let canonical = canonicalize_value(&Value::Object(unsigned_bundle.clone()));
    let payload = serde_json::to_vec(&canonical).context("failed serializing canonical bundle")?;

    let mut mac = HmacSha256::new_from_slice(key.as_bytes())
        .context("failed initializing policy bundle verifier")?;
    mac.update(&payload);
    let computed = bytes_to_hex_lowercase(&mac.finalize().into_bytes());

    let normalized_provided = normalize_signature(provided_signature);
    if computed != normalized_provided {
        return Err(anyhow!("policy bundle signature verification failed"));
    }
    Ok(())
}

fn normalize_signature(value: &str) -> String {
    let trimmed = value.trim().to_ascii_lowercase();
    trimmed
        .strip_prefix("sha256:")
        .map(ToOwned::to_owned)
        .unwrap_or(trimmed)
}

fn bytes_to_hex_lowercase(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(hex_nibble((byte >> 4) & 0x0f));
        output.push(hex_nibble(byte & 0x0f));
    }
    output
}

fn hex_nibble(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + (value - 10)) as char,
        _ => '0',
    }
}

fn canonicalize_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut ordered = BTreeMap::new();
            for (key, val) in map {
                ordered.insert(key.clone(), canonicalize_value(val));
            }
            let mut out = Map::new();
            for (key, val) in ordered {
                out.insert(key, val);
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_value).collect()),
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::{
        apply_signed_policy_bundle, bytes_to_hex_lowercase, canonicalize_value, HmacSha256,
    };
    use crate::config::{Config, PolicyAction};
    use hmac::Mac;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        path.push(format!("openclaw-rs-policy-{name}-{stamp}.json"));
        path
    }

    fn sign_unsigned_bundle(unsigned: serde_json::Value, key: &str) -> serde_json::Value {
        let canonical = canonicalize_value(&unsigned);
        let payload = serde_json::to_vec(&canonical).expect("serialize");
        let mut mac = HmacSha256::new_from_slice(key.as_bytes()).expect("mac");
        mac.update(&payload);
        let signature = bytes_to_hex_lowercase(&mac.finalize().into_bytes());

        let mut root = unsigned.as_object().cloned().expect("unsigned object");
        root.insert("signature".to_owned(), json!(signature));
        serde_json::Value::Object(root)
    }

    #[tokio::test]
    async fn loads_valid_signed_bundle_and_applies_policy_patch() {
        let path = temp_path("valid");
        let key = "policy-secret";
        let bundle = sign_unsigned_bundle(
            json!({
                "version": 1,
                "bundleId": "ops-bundle",
                "signedAt": "2026-02-18T00:00:00Z",
                "policy": {
                    "reviewThreshold": 25,
                    "blockThreshold": 55,
                    "allowedCommandPrefixes": ["git ", "rg "],
                    "blockedCommandPatterns": ["(?i)rm\\s+-rf\\s+/"],
                    "promptInjectionPatterns": ["(?i)ignore\\s+all\\s+previous\\s+instructions"],
                    "toolPolicies": {"gateway": "block"},
                    "toolRiskBonus": {"exec": 40},
                    "channelRiskBonus": {"discord": 15}
                }
            }),
            key,
        );
        tokio::fs::write(
            &path,
            serde_json::to_vec_pretty(&bundle).expect("bundle json"),
        )
        .await
        .expect("write bundle");

        let mut cfg = Config::default();
        cfg.security.policy_bundle_path = Some(path.clone());
        cfg.security.policy_bundle_key = Some(key.to_owned());

        let report = apply_signed_policy_bundle(&mut cfg)
            .await
            .expect("bundle load")
            .expect("bundle report");

        assert_eq!(report.version, 1);
        assert_eq!(report.bundle_id.as_deref(), Some("ops-bundle"));
        assert_eq!(report.path, path);
        assert_eq!(cfg.security.review_threshold, 25);
        assert_eq!(cfg.security.block_threshold, 55);
        assert_eq!(
            cfg.security.tool_policies.get("gateway"),
            Some(&PolicyAction::Block)
        );
        assert_eq!(cfg.security.tool_risk_bonus.get("exec"), Some(&40));
        assert_eq!(cfg.security.channel_risk_bonus.get("discord"), Some(&15));
    }

    #[tokio::test]
    async fn rejects_bundle_with_invalid_signature() {
        let path = temp_path("bad-signature");
        let bundle = json!({
            "version": 1,
            "policy": {"reviewThreshold": 20, "blockThreshold": 60},
            "signature": "deadbeef"
        });
        tokio::fs::write(
            &path,
            serde_json::to_vec_pretty(&bundle).expect("bundle json"),
        )
        .await
        .expect("write bundle");

        let mut cfg = Config::default();
        cfg.security.policy_bundle_path = Some(path);
        cfg.security.policy_bundle_key = Some("policy-secret".to_owned());

        let err = apply_signed_policy_bundle(&mut cfg)
            .await
            .expect_err("bundle should fail");
        assert!(
            err.to_string().contains("signature verification failed"),
            "unexpected error: {err:#}"
        );
    }

    #[tokio::test]
    async fn rejects_bundle_without_key() {
        let path = temp_path("missing-key");
        let bundle = json!({
            "version": 1,
            "policy": {"reviewThreshold": 20, "blockThreshold": 60},
            "signature": "deadbeef"
        });
        tokio::fs::write(
            &path,
            serde_json::to_vec_pretty(&bundle).expect("bundle json"),
        )
        .await
        .expect("write bundle");

        let mut cfg = Config::default();
        cfg.security.policy_bundle_path = Some(path);
        cfg.security.policy_bundle_key = None;

        let err = apply_signed_policy_bundle(&mut cfg)
            .await
            .expect_err("bundle should fail");
        assert!(
            err.to_string().contains("policy bundle key missing"),
            "unexpected error: {err:#}"
        );
    }
}
