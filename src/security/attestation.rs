use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::config::Config;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone)]
pub struct AttestationAlert {
    pub risk_bonus: u8,
    pub tag: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeAttestationSnapshot {
    pub executable_path: String,
    pub executable_sha256: String,
    pub started_at_ms: u64,
    pub expected_sha256: Option<String>,
    pub verified: bool,
    pub signature: Option<String>,
}

pub struct RuntimeAttestationGuard {
    snapshot: RuntimeAttestationSnapshot,
    mismatch_risk_bonus: u8,
}

impl RuntimeAttestationGuard {
    pub async fn new(cfg: &Config) -> Result<Self> {
        let expected_sha256 = cfg
            .security
            .attestation_expected_sha256
            .as_deref()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty());
        let mismatch_risk_bonus = cfg.security.attestation_mismatch_risk_bonus.max(1);

        let (path, executable_sha256) = compute_current_executable_sha256().await?;
        let started_at_ms = now_ms();
        let executable_path = path.display().to_string();
        let verified = expected_sha256
            .as_deref()
            .is_none_or(|expected| expected == executable_sha256);
        let signature = sign_attestation_payload(
            cfg.security.attestation_hmac_key.as_deref(),
            started_at_ms,
            executable_path.as_str(),
            executable_sha256.as_str(),
        )?;

        let snapshot = RuntimeAttestationSnapshot {
            executable_path,
            executable_sha256,
            started_at_ms,
            expected_sha256,
            verified,
            signature,
        };
        if let Some(path) = cfg.security.attestation_report_path.as_ref() {
            persist_attestation_report(path, &snapshot).await?;
        }
        Ok(Self {
            snapshot,
            mismatch_risk_bonus,
        })
    }

    pub fn snapshot(&self) -> RuntimeAttestationSnapshot {
        self.snapshot.clone()
    }

    pub fn mismatch_alert(&self) -> Option<AttestationAlert> {
        if self.snapshot.verified {
            return None;
        }
        let expected = self
            .snapshot
            .expected_sha256
            .as_deref()
            .unwrap_or("unset-expected-sha256");
        Some(AttestationAlert {
            risk_bonus: self.mismatch_risk_bonus,
            tag: "runtime_attestation_mismatch".to_owned(),
            reason: format!(
                "runtime binary digest mismatch: expected {}, actual {}",
                expected, self.snapshot.executable_sha256
            ),
        })
    }
}

async fn compute_current_executable_sha256() -> Result<(PathBuf, String)> {
    let path = std::env::current_exe().context("resolve current executable path")?;
    let path_for_read = path.clone();
    let digest = tokio::task::spawn_blocking(move || -> Result<String> {
        let bytes = std::fs::read(&path_for_read)
            .with_context(|| format!("read executable {}", path_for_read.display()))?;
        let hash = Sha256::digest(&bytes);
        Ok(hex_encode(hash.as_slice()))
    })
    .await
    .context("join executable digest task")??;
    Ok((path, digest))
}

fn sign_attestation_payload(
    key: Option<&str>,
    started_at_ms: u64,
    executable_path: &str,
    executable_sha256: &str,
) -> Result<Option<String>> {
    let Some(key) = key.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let payload = format!("{started_at_ms}:{executable_path}:{executable_sha256}");
    let mut mac =
        HmacSha256::new_from_slice(key.as_bytes()).context("initialize attestation hmac signer")?;
    mac.update(payload.as_bytes());
    let bytes = mac.finalize().into_bytes();
    Ok(Some(
        base64::engine::general_purpose::STANDARD.encode(bytes),
    ))
}

async fn persist_attestation_report(
    report_path: &PathBuf,
    snapshot: &RuntimeAttestationSnapshot,
) -> Result<()> {
    if let Some(parent) = report_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create attestation report directory {}", parent.display()))?;
    }
    let payload = serde_json::to_vec_pretty(snapshot).context("serialize attestation report")?;
    tokio::fs::write(report_path, payload)
        .await
        .with_context(|| format!("write attestation report {}", report_path.display()))?;
    Ok(())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
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

    use crate::config::Config;

    use super::RuntimeAttestationGuard;

    #[tokio::test]
    async fn attestation_alerts_on_expected_digest_mismatch() {
        let mut cfg = Config::default();
        cfg.security.attestation_expected_sha256 = Some("0".repeat(64));
        let guard = RuntimeAttestationGuard::new(&cfg)
            .await
            .expect("attestation guard");
        let alert = guard.mismatch_alert().expect("mismatch alert");
        assert_eq!(alert.tag, "runtime_attestation_mismatch");
        assert!(alert.risk_bonus >= 1);
    }

    #[tokio::test]
    async fn attestation_writes_signed_report_when_configured() {
        let mut cfg = Config::default();
        let mut report_path = std::env::temp_dir();
        report_path.push(format!(
            "openclaw-rs-attestation-report-{}.json",
            super::now_ms()
        ));
        cfg.security.attestation_report_path = Some(report_path.clone());
        cfg.security.attestation_hmac_key = Some("test-hmac-key".to_owned());
        cfg.security.attestation_expected_sha256 = None;

        let guard = RuntimeAttestationGuard::new(&cfg)
            .await
            .expect("attestation guard");
        assert!(guard.mismatch_alert().is_none());

        let payload = tokio::fs::read_to_string(&report_path)
            .await
            .expect("report file");
        let report: serde_json::Value = serde_json::from_str(&payload).expect("json");
        assert!(report
            .get("signature")
            .and_then(serde_json::Value::as_str)
            .is_some());
        let _ = tokio::fs::remove_file(PathBuf::from(&report_path)).await;
    }
}
