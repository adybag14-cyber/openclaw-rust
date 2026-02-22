use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::config::ToolRuntimeWasmPolicyConfig;

#[derive(Debug, Clone, Serialize)]
pub struct WasmInspection {
    pub module: String,
    pub module_path: String,
    pub module_size_bytes: u64,
    pub module_sha256: String,
    pub requested_capabilities: Vec<String>,
    pub granted_capabilities: Vec<String>,
    pub blocked_capabilities: Vec<String>,
    pub fuel_limit: u64,
    pub memory_limit_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct WasmSandbox {
    cfg: ToolRuntimeWasmPolicyConfig,
}

impl WasmSandbox {
    pub fn new(cfg: ToolRuntimeWasmPolicyConfig) -> Self {
        Self { cfg }
    }

    pub fn enabled(&self) -> bool {
        self.cfg.enabled
    }

    pub fn inspect(&self, module: &str, requested: &[String]) -> Result<WasmInspection> {
        let module_root = canonicalize_or_create(&self.cfg.module_root)?;
        let resolved = resolve_path_inside_root(&module_root, module)?;
        let payload = std::fs::read(&resolved)
            .with_context(|| format!("failed reading wasm module {}", resolved.display()))?;

        let mut hasher = Sha256::new();
        hasher.update(&payload);
        let digest = hasher.finalize();

        let requested_caps = normalize_list(requested);
        let mut allowed_caps = normalize_list(&self.cfg.default_capabilities);
        if let Some(extra) = self.cfg.module_capabilities.get(module) {
            allowed_caps.extend(normalize_list(extra));
        }
        if let Some(file_name) = resolved.file_name().and_then(|entry| entry.to_str()) {
            if let Some(extra) = self.cfg.module_capabilities.get(file_name) {
                allowed_caps.extend(normalize_list(extra));
            }
        }

        let granted = if requested_caps.is_empty() {
            allowed_caps.clone()
        } else {
            requested_caps
                .iter()
                .filter(|entry| allowed_caps.contains(*entry))
                .cloned()
                .collect::<Vec<_>>()
        };
        let blocked = requested_caps
            .iter()
            .filter(|entry| !allowed_caps.contains(*entry))
            .cloned()
            .collect::<Vec<_>>();

        Ok(WasmInspection {
            module: module.to_owned(),
            module_path: resolved.display().to_string(),
            module_size_bytes: payload.len() as u64,
            module_sha256: format!("{digest:x}"),
            requested_capabilities: requested_caps,
            granted_capabilities: granted,
            blocked_capabilities: blocked,
            fuel_limit: self.cfg.fuel_limit,
            memory_limit_bytes: self.cfg.memory_limit_bytes,
        })
    }
}

fn canonicalize_or_create(path: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(path)
        .with_context(|| format!("failed creating wasm module root {}", path.display()))?;
    std::fs::canonicalize(path)
        .with_context(|| format!("failed canonicalizing wasm module root {}", path.display()))
}

fn resolve_path_inside_root(root: &Path, raw: &str) -> Result<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("module path must be non-empty");
    }

    let candidate = if Path::new(trimmed).is_absolute() {
        PathBuf::from(trimmed)
    } else {
        root.join(trimmed)
    };
    let resolved = std::fs::canonicalize(&candidate)
        .with_context(|| format!("failed resolving module path {}", candidate.display()))?;
    if !resolved.starts_with(root) {
        anyhow::bail!(
            "module path `{}` escapes wasm module root {}",
            raw,
            root.display()
        );
    }
    Ok(resolved)
}

fn normalize_list(values: &[String]) -> Vec<String> {
    let mut unique = BTreeSet::new();
    for value in values {
        let normalized = value.trim().to_ascii_lowercase();
        if !normalized.is_empty() {
            unique.insert(normalized);
        }
    }
    unique.into_iter().collect()
}
