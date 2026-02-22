use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize)]
pub struct RegisteredWitTool {
    pub id: String,
    pub package: Option<String>,
    pub source_path: String,
    pub worlds: Vec<String>,
    pub interfaces: Vec<String>,
    pub operations: Vec<String>,
    pub schema: Value,
}

#[derive(Debug, Clone)]
pub struct ToolRegistry {
    root: PathBuf,
    dynamic_wit_loading: bool,
    tools: BTreeMap<String, RegisteredWitTool>,
}

impl ToolRegistry {
    pub fn new(root: PathBuf, dynamic_wit_loading: bool) -> Self {
        Self {
            root,
            dynamic_wit_loading,
            tools: BTreeMap::new(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn dynamic_wit_loading(&self) -> bool {
        self.dynamic_wit_loading
    }

    pub fn refresh(&mut self) -> Result<usize> {
        self.tools.clear();
        if !self.root.exists() {
            return Ok(0);
        }

        let files = collect_wit_files(&self.root)?;
        for file in files {
            if let Ok(tool) = parse_wit_file(&self.root, &file) {
                self.tools.insert(tool.id.clone(), tool);
            }
        }
        Ok(self.tools.len())
    }

    pub fn list(&self) -> Vec<RegisteredWitTool> {
        self.tools.values().cloned().collect()
    }

    pub fn schema(&self, id: &str) -> Option<Value> {
        self.tools.get(id).map(|entry| entry.schema.clone())
    }

    pub fn resolve(&self, id_or_module: &str) -> Option<RegisteredWitTool> {
        let normalized = normalize_name(id_or_module);
        if let Some(tool) = self.tools.get(&normalized) {
            return Some(tool.clone());
        }

        let module_name = Path::new(id_or_module)
            .file_stem()
            .and_then(|entry| entry.to_str())
            .map(normalize_name);
        module_name.and_then(|module| self.tools.get(&module).cloned())
    }
}

fn collect_wit_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current)
            .with_context(|| format!("failed listing WIT directory {}", current.display()))?;
        for entry in entries {
            let entry = entry.with_context(|| "failed reading WIT directory entry")?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("wit"))
            {
                out.push(path);
            }
        }
    }

    out.sort();
    Ok(out)
}

fn parse_wit_file(root: &Path, path: &Path) -> Result<RegisteredWitTool> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed reading WIT file {}", path.display()))?;

    let mut package = None;
    let mut worlds = BTreeSet::new();
    let mut interfaces = BTreeSet::new();
    let mut operations = BTreeSet::new();

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if package.is_none() && line.starts_with("package ") {
            let value = line
                .trim_start_matches("package ")
                .trim()
                .trim_end_matches(';');
            if !value.is_empty() {
                package = Some(value.to_owned());
            }
            continue;
        }

        if let Some(name) = extract_decl_name(line, "world ") {
            worlds.insert(name);
            continue;
        }
        if let Some(name) = extract_decl_name(line, "interface ") {
            interfaces.insert(name);
            continue;
        }
        if let Some(name) = extract_operation_name(line) {
            operations.insert(name);
        }
    }

    let source_path = path
        .strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
        .replace('\\', "/");
    let file_stem = path
        .file_stem()
        .and_then(|entry| entry.to_str())
        .unwrap_or("tool");
    let id_seed = worlds
        .iter()
        .next()
        .cloned()
        .or_else(|| interfaces.iter().next().cloned())
        .unwrap_or_else(|| file_stem.to_owned());
    let id = normalize_name(&id_seed);
    let worlds = worlds.into_iter().collect::<Vec<_>>();
    let interfaces = interfaces.into_iter().collect::<Vec<_>>();
    let operations = operations.into_iter().collect::<Vec<_>>();
    let schema = build_schema(&id, &operations, &source_path, package.as_deref());

    Ok(RegisteredWitTool {
        id,
        package,
        source_path,
        worlds,
        interfaces,
        operations,
        schema,
    })
}

fn extract_decl_name(line: &str, prefix: &str) -> Option<String> {
    if !line.starts_with(prefix) {
        return None;
    }
    let value = line.trim_start_matches(prefix).trim();
    let boundary = value.find([' ', '{', ';', ':']).unwrap_or(value.len());
    let name = value[..boundary].trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_owned())
    }
}

fn extract_operation_name(line: &str) -> Option<String> {
    let (left, right) = line.split_once(':')?;
    if !right.trim_start().starts_with("func") {
        return None;
    }
    let name = left.trim();
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return None;
    }
    Some(name.to_owned())
}

fn build_schema(
    id: &str,
    operations: &[String],
    source_path: &str,
    package: Option<&str>,
) -> Value {
    let mut properties = serde_json::Map::new();
    properties.insert(
        "action".to_owned(),
        json!({
            "type": "string",
            "enum": ["inspect", "execute"]
        }),
    );
    properties.insert("module".to_owned(), json!({ "type": "string" }));
    properties.insert("payload".to_owned(), json!({}));
    if operations.is_empty() {
        properties.insert("operation".to_owned(), json!({ "type": "string" }));
    } else {
        properties.insert(
            "operation".to_owned(),
            json!({
                "type": "string",
                "enum": operations
            }),
        );
    }

    let mut schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": format!("WIT Tool {id}"),
        "type": "object",
        "required": ["module"],
        "additionalProperties": true,
        "properties": Value::Object(properties),
        "x-openclaw-source": source_path,
    });
    if let Some(package) = package {
        if let Value::Object(ref mut map) = schema {
            map.insert("x-openclaw-package".to_owned(), json!(package));
        }
    }
    schema
}

fn normalize_name(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(' ', "-")
}
