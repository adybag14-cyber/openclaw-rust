#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::config::ToolRuntimePolicyConfig;
use crate::security::tool_loop::{ToolLoopGuard, ToolLoopLevel};
use crate::security::tool_policy::ToolPolicyMatcher;
use crate::types::ActionRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRuntimeErrorCode {
    InvalidArgs,
    UnsupportedTool,
    PolicyDenied,
    LoopCritical,
    PathOutsideRoot,
    Io,
    ExecutionFailed,
    ProcessNotFound,
}

impl ToolRuntimeErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArgs => "invalid_args",
            Self::UnsupportedTool => "unsupported_tool",
            Self::PolicyDenied => "policy_denied",
            Self::LoopCritical => "loop_critical",
            Self::PathOutsideRoot => "path_outside_root",
            Self::Io => "io_error",
            Self::ExecutionFailed => "execution_failed",
            Self::ProcessNotFound => "process_not_found",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRuntimeError {
    pub code: ToolRuntimeErrorCode,
    pub message: String,
}

impl ToolRuntimeError {
    fn new(code: ToolRuntimeErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

type ToolRuntimeResult<T> = Result<T, ToolRuntimeError>;

#[derive(Debug, Clone, Deserialize)]
pub struct ToolRuntimeRequest {
    pub request_id: String,
    pub session_id: String,
    #[serde(alias = "tool", alias = "toolName")]
    pub tool_name: String,
    #[serde(default)]
    pub args: Value,
    #[serde(default)]
    pub sandboxed: bool,
    #[serde(default)]
    pub model_provider: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolRuntimeResponse {
    pub result: Value,
    pub warnings: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ToolTranscriptEntry {
    pub request_id: String,
    pub session_id: String,
    pub tool_name: String,
    pub sandboxed: bool,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
    pub status: &'static str,
    pub warnings: Vec<String>,
    pub error_code: Option<&'static str>,
}

#[derive(Debug)]
struct ToolRuntimeProcessOutcome {
    status: &'static str,
    exit_code: Option<i32>,
    aggregated: String,
    duration_ms: u64,
}

enum ToolRuntimeProcessExecution {
    Running(JoinHandle<ToolRuntimeResult<ToolRuntimeProcessOutcome>>),
    Completed(ToolRuntimeProcessOutcome),
    Failed(String),
}

struct ToolRuntimeProcessSession {
    session_id: String,
    command: String,
    cwd: String,
    started_at_ms: u64,
    execution: ToolRuntimeProcessExecution,
}

#[derive(Debug, Clone)]
struct ToolRuntimeSessionEntry {
    id: String,
    role: String,
    message: String,
    created_at_ms: u64,
}

#[derive(Debug, Clone, Default)]
struct ToolRuntimeSessionTimeline {
    entries: VecDeque<ToolRuntimeSessionEntry>,
    updated_at_ms: u64,
}

pub struct ToolRuntimeHost {
    workspace_root: PathBuf,
    sandbox_root: PathBuf,
    policy: ToolPolicyMatcher,
    loop_guard: ToolLoopGuard,
    transcript_limit: usize,
    session_history_limit: usize,
    session_bucket_limit: usize,
    transcript: Mutex<VecDeque<ToolTranscriptEntry>>,
    process_counter: Mutex<u64>,
    session_entry_counter: Mutex<u64>,
    process_sessions: Mutex<HashMap<String, ToolRuntimeProcessSession>>,
    session_timelines: Mutex<HashMap<String, ToolRuntimeSessionTimeline>>,
}

impl ToolRuntimeHost {
    pub async fn new(
        workspace_root: PathBuf,
        sandbox_root: PathBuf,
        policy: ToolRuntimePolicyConfig,
    ) -> ToolRuntimeResult<Arc<Self>> {
        std::fs::create_dir_all(&workspace_root).map_err(|err| {
            ToolRuntimeError::new(
                ToolRuntimeErrorCode::Io,
                format!(
                    "failed creating workspace root {}: {err}",
                    workspace_root.display()
                ),
            )
        })?;
        std::fs::create_dir_all(&sandbox_root).map_err(|err| {
            ToolRuntimeError::new(
                ToolRuntimeErrorCode::Io,
                format!(
                    "failed creating sandbox root {}: {err}",
                    sandbox_root.display()
                ),
            )
        })?;

        let workspace_root = canonicalize_path_lossy(&workspace_root)?;
        let sandbox_root = canonicalize_path_lossy(&sandbox_root)?;

        let policy_matcher = ToolPolicyMatcher::new(policy.clone());
        let loop_guard = ToolLoopGuard::new(policy.loop_detection);

        Ok(Arc::new(Self {
            workspace_root,
            sandbox_root,
            policy: policy_matcher,
            loop_guard,
            transcript_limit: 512,
            session_history_limit: 256,
            session_bucket_limit: 256,
            transcript: Mutex::new(VecDeque::new()),
            process_counter: Mutex::new(0),
            session_entry_counter: Mutex::new(0),
            process_sessions: Mutex::new(HashMap::new()),
            session_timelines: Mutex::new(HashMap::new()),
        }))
    }

    pub async fn execute(
        &self,
        request: ToolRuntimeRequest,
    ) -> ToolRuntimeResult<ToolRuntimeResponse> {
        let started_at_ms = now_ms();
        let normalized_tool = normalize_tool_name(&request.tool_name);
        if normalized_tool.is_empty() {
            let err = ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                "tool_name must be a non-empty string",
            );
            self.record_transcript(started_at_ms, &request, &normalized_tool, &[], Some(&err))
                .await;
            return Err(err);
        }

        if !self.policy.allows(
            &normalized_tool,
            request.model_provider.as_deref(),
            request.model_id.as_deref(),
        ) {
            let err = ToolRuntimeError::new(
                ToolRuntimeErrorCode::PolicyDenied,
                format!("tool `{normalized_tool}` denied by runtime policy"),
            );
            self.record_transcript(started_at_ms, &request, &normalized_tool, &[], Some(&err))
                .await;
            return Err(err);
        }

        let mut warnings = Vec::new();
        if let Some(alert) = self.observe_loop_guard(&request, &normalized_tool).await? {
            match alert.level {
                ToolLoopLevel::Warning => {
                    warnings.push(format!(
                        "tool loop warning: `{}` repeated {} times with identical arguments",
                        normalized_tool, alert.count
                    ));
                }
                ToolLoopLevel::Critical => {
                    let err = ToolRuntimeError::new(
                        ToolRuntimeErrorCode::LoopCritical,
                        format!(
                            "tool loop critical: `{}` repeated {} times with identical arguments",
                            normalized_tool, alert.count
                        ),
                    );
                    self.record_transcript(
                        started_at_ms,
                        &request,
                        &normalized_tool,
                        &warnings,
                        Some(&err),
                    )
                    .await;
                    return Err(err);
                }
            }
        }

        let result = self.execute_inner(&request, &normalized_tool).await;
        match result {
            Ok(result) => {
                self.record_transcript(started_at_ms, &request, &normalized_tool, &warnings, None)
                    .await;
                Ok(ToolRuntimeResponse { result, warnings })
            }
            Err(err) => {
                self.record_transcript(
                    started_at_ms,
                    &request,
                    &normalized_tool,
                    &warnings,
                    Some(&err),
                )
                .await;
                Err(err)
            }
        }
    }

    #[cfg(test)]
    pub async fn transcript(&self) -> Vec<ToolTranscriptEntry> {
        self.transcript.lock().await.iter().cloned().collect()
    }

    async fn observe_loop_guard(
        &self,
        request: &ToolRuntimeRequest,
        tool_name: &str,
    ) -> ToolRuntimeResult<Option<crate::security::tool_loop::ToolLoopAlert>> {
        let action_request = ActionRequest {
            id: request.request_id.clone(),
            source: "tool_runtime".to_owned(),
            session_id: Some(request.session_id.clone()),
            prompt: request
                .args
                .get("prompt")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            command: request
                .args
                .get("command")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            tool_name: Some(tool_name.to_owned()),
            channel: None,
            url: request
                .args
                .get("url")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            file_path: first_string_arg(&request.args, &["path", "file_path"]),
            raw: json!({
                "tool": tool_name,
                "args": request.args
            }),
        };
        Ok(self.loop_guard.observe(&action_request).await)
    }

    async fn execute_inner(
        &self,
        request: &ToolRuntimeRequest,
        tool_name: &str,
    ) -> ToolRuntimeResult<Value> {
        match tool_name {
            "read" => self.execute_read(request).await,
            "write" => self.execute_write(request).await,
            "edit" => self.execute_edit(request).await,
            "apply_patch" => self.execute_apply_patch(request).await,
            "exec" => self.execute_exec(request).await,
            "process" => self.execute_process(request).await,
            "gateway" => self.execute_gateway(request).await,
            "sessions" => self.execute_sessions(request).await,
            "message" => self.execute_message(request).await,
            "browser" => self.execute_browser(request).await,
            "canvas" => self.execute_canvas(request).await,
            "nodes" => self.execute_nodes(request).await,
            _ => Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::UnsupportedTool,
                format!("unsupported tool `{tool_name}`"),
            )),
        }
    }

    fn root_for_request(&self, request: &ToolRuntimeRequest) -> &Path {
        if request.sandboxed {
            &self.sandbox_root
        } else {
            &self.workspace_root
        }
    }

    async fn execute_read(&self, request: &ToolRuntimeRequest) -> ToolRuntimeResult<Value> {
        let path = required_string_arg(&request.args, &["path", "file_path"], "path")?;
        let root = self.root_for_request(request);
        let resolved = resolve_path_inside_root(root, &path)?;
        let metadata = std::fs::metadata(&resolved).map_err(|err| {
            ToolRuntimeError::new(
                ToolRuntimeErrorCode::Io,
                format!("failed reading metadata for {}: {err}", resolved.display()),
            )
        })?;
        if !metadata.is_file() {
            return Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                format!("path is not a file: {}", display_path(root, &resolved)),
            ));
        }
        let content = std::fs::read_to_string(&resolved).map_err(|err| {
            ToolRuntimeError::new(
                ToolRuntimeErrorCode::Io,
                format!("failed reading file {}: {err}", resolved.display()),
            )
        })?;
        Ok(json!({
            "status": "completed",
            "path": display_path(root, &resolved),
            "content": content,
            "bytes": metadata.len()
        }))
    }

    async fn execute_write(&self, request: &ToolRuntimeRequest) -> ToolRuntimeResult<Value> {
        let path = required_string_arg(&request.args, &["path", "file_path"], "path")?;
        let content = required_string_arg(&request.args, &["content"], "content")?;
        let root = self.root_for_request(request);
        let resolved = resolve_path_inside_root(root, &path)?;
        if let Some(parent) = resolved.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                ToolRuntimeError::new(
                    ToolRuntimeErrorCode::Io,
                    format!(
                        "failed creating parent directory {}: {err}",
                        parent.display()
                    ),
                )
            })?;
        }
        std::fs::write(&resolved, content.as_bytes()).map_err(|err| {
            ToolRuntimeError::new(
                ToolRuntimeErrorCode::Io,
                format!("failed writing file {}: {err}", resolved.display()),
            )
        })?;
        Ok(json!({
            "status": "completed",
            "path": display_path(root, &resolved),
            "bytesWritten": content.len()
        }))
    }

    async fn execute_edit(&self, request: &ToolRuntimeRequest) -> ToolRuntimeResult<Value> {
        let path = required_string_arg(&request.args, &["path", "file_path"], "path")?;
        let old_text = required_string_arg(&request.args, &["oldText", "old_string"], "oldText")?;
        let new_text = required_string_arg(&request.args, &["newText", "new_string"], "newText")?;
        let root = self.root_for_request(request);
        let resolved = resolve_path_inside_root(root, &path)?;
        let original = std::fs::read_to_string(&resolved).map_err(|err| {
            ToolRuntimeError::new(
                ToolRuntimeErrorCode::Io,
                format!("failed reading file {}: {err}", resolved.display()),
            )
        })?;

        let Some(index) = original.find(&old_text) else {
            return Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::ExecutionFailed,
                format!(
                    "edit failed: oldText not found in {}",
                    display_path(root, &resolved)
                ),
            ));
        };

        let mut updated = String::with_capacity(original.len() + new_text.len());
        updated.push_str(&original[..index]);
        updated.push_str(&new_text);
        updated.push_str(&original[index + old_text.len()..]);
        std::fs::write(&resolved, updated.as_bytes()).map_err(|err| {
            ToolRuntimeError::new(
                ToolRuntimeErrorCode::Io,
                format!("failed writing file {}: {err}", resolved.display()),
            )
        })?;

        Ok(json!({
            "status": "completed",
            "path": display_path(root, &resolved),
            "replaced": 1
        }))
    }

    async fn execute_apply_patch(&self, request: &ToolRuntimeRequest) -> ToolRuntimeResult<Value> {
        let patch_input = required_string_arg(&request.args, &["input"], "input")?;
        let root = self.root_for_request(request);
        let parsed = parse_patch_text(&patch_input)?;
        let summary = apply_patch_hunks(root, &parsed)?;
        Ok(json!({
            "status": "completed",
            "summary": {
                "added": summary.added,
                "modified": summary.modified,
                "deleted": summary.deleted
            }
        }))
    }

    async fn execute_exec(&self, request: &ToolRuntimeRequest) -> ToolRuntimeResult<Value> {
        let command = required_string_arg(&request.args, &["command"], "command")?;
        let background = request
            .args
            .get("background")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let root = self.root_for_request(request);
        let cwd = match first_string_arg(&request.args, &["workdir", "cwd"]) {
            Some(raw) => {
                let resolved = resolve_path_inside_root(root, &raw)?;
                if !resolved.is_dir() {
                    return Err(ToolRuntimeError::new(
                        ToolRuntimeErrorCode::InvalidArgs,
                        format!(
                            "workdir is not a directory: {}",
                            display_path(root, &resolved)
                        ),
                    ));
                }
                resolved
            }
            None => root.to_path_buf(),
        };

        if background {
            let session_id = self.next_process_session_id().await;
            let session_cwd = cwd.clone();
            let command_text = command.clone();
            let started = now_ms();
            let handle =
                tokio::spawn(
                    async move { run_shell_command(command_text, session_cwd.clone()).await },
                );

            let mut sessions = self.process_sessions.lock().await;
            sessions.insert(
                session_id.clone(),
                ToolRuntimeProcessSession {
                    session_id: session_id.clone(),
                    command,
                    cwd: cwd.display().to_string(),
                    started_at_ms: started,
                    execution: ToolRuntimeProcessExecution::Running(handle),
                },
            );
            return Ok(json!({
                "status": "running",
                "sessionId": session_id,
                "cwd": display_path(root, &cwd)
            }));
        }

        let outcome = run_shell_command(command, cwd.clone()).await?;
        Ok(json!({
            "status": outcome.status,
            "exitCode": outcome.exit_code,
            "durationMs": outcome.duration_ms,
            "aggregated": outcome.aggregated,
            "cwd": display_path(root, &cwd)
        }))
    }

    async fn execute_process(&self, request: &ToolRuntimeRequest) -> ToolRuntimeResult<Value> {
        let action = request
            .args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("list")
            .to_ascii_lowercase();

        match action.as_str() {
            "list" => {
                let sessions = self.process_sessions.lock().await;
                let rows = sessions
                    .values()
                    .map(|session| {
                        let status = match session.execution {
                            ToolRuntimeProcessExecution::Running(_) => "running",
                            ToolRuntimeProcessExecution::Completed(ref outcome) => outcome.status,
                            ToolRuntimeProcessExecution::Failed(_) => "failed",
                        };
                        json!({
                            "sessionId": session.session_id,
                            "status": status,
                            "startedAt": session.started_at_ms,
                            "cwd": session.cwd,
                            "command": session.command
                        })
                    })
                    .collect::<Vec<_>>();
                Ok(json!({
                    "status": "completed",
                    "sessions": rows
                }))
            }
            "poll" | "log" | "kill" | "remove" => {
                let session_id =
                    required_string_arg(&request.args, &["sessionId", "session_id"], "sessionId")?;
                let mut session = {
                    let mut sessions = self.process_sessions.lock().await;
                    sessions.remove(&session_id).ok_or_else(|| {
                        ToolRuntimeError::new(
                            ToolRuntimeErrorCode::ProcessNotFound,
                            format!("no process session found for {session_id}"),
                        )
                    })?
                };

                self.refresh_process_session(&mut session).await;

                let payload = match action.as_str() {
                    "poll" => process_poll_payload(&session),
                    "log" => process_log_payload(&session),
                    "kill" => {
                        if let ToolRuntimeProcessExecution::Running(handle) = &session.execution {
                            handle.abort();
                            session.execution = ToolRuntimeProcessExecution::Failed(
                                "killed by process.kill".to_owned(),
                            );
                        }
                        process_poll_payload(&session)
                    }
                    "remove" => {
                        if let ToolRuntimeProcessExecution::Running(handle) = &session.execution {
                            handle.abort();
                        }
                        json!({
                            "status": "completed",
                            "removed": true,
                            "sessionId": session_id
                        })
                    }
                    _ => unreachable!(),
                };

                if action != "remove" {
                    let mut sessions = self.process_sessions.lock().await;
                    sessions.insert(session_id, session);
                }

                Ok(payload)
            }
            _ => Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                format!("unsupported process action `{action}`"),
            )),
        }
    }

    async fn execute_gateway(&self, request: &ToolRuntimeRequest) -> ToolRuntimeResult<Value> {
        let action = request
            .args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("status")
            .to_ascii_lowercase();

        const TOOLS: &[&str] = &[
            "read",
            "write",
            "edit",
            "apply_patch",
            "exec",
            "process",
            "gateway",
            "sessions",
            "message",
            "browser",
            "canvas",
            "nodes",
        ];

        match action.as_str() {
            "status" => Ok(json!({
                "status": "completed",
                "connected": true,
                "workspaceRoot": self.workspace_root.display().to_string(),
                "sandboxRoot": self.sandbox_root.display().to_string(),
                "capabilities": {
                    "tools": TOOLS,
                    "sessionHistoryLimit": self.session_history_limit,
                    "sessionBucketLimit": self.session_bucket_limit
                },
                "ts": now_ms()
            })),
            "health" => Ok(json!({
                "status": "ok",
                "ok": true,
                "ts": now_ms()
            })),
            "methods" | "tools" => Ok(json!({
                "status": "completed",
                "methods": TOOLS,
                "count": TOOLS.len()
            })),
            _ => Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                format!("unsupported gateway action `{action}`"),
            )),
        }
    }

    async fn execute_sessions(&self, request: &ToolRuntimeRequest) -> ToolRuntimeResult<Value> {
        let action = request
            .args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("history")
            .to_ascii_lowercase();
        match action.as_str() {
            "send" | "append" => {
                let message =
                    first_string_arg(&request.args, &["message", "content", "text", "prompt"])
                        .ok_or_else(|| {
                            ToolRuntimeError::new(
                                ToolRuntimeErrorCode::InvalidArgs,
                                "missing required parameter `message`",
                            )
                        })?;
                let session_id = first_string_arg(&request.args, &["sessionId", "session_id"])
                    .unwrap_or_else(|| request.session_id.clone());
                let role = normalize_message_role(
                    first_string_arg(&request.args, &["role", "author", "sender"]).as_deref(),
                );
                let (entry, count) = self
                    .append_session_entry(session_id.clone(), role, message)
                    .await;
                Ok(json!({
                    "status": "completed",
                    "sessionId": session_id,
                    "entry": entry,
                    "count": count
                }))
            }
            "history" => {
                let session_id = first_string_arg(&request.args, &["sessionId", "session_id"])
                    .unwrap_or_else(|| request.session_id.clone());
                let (entries, count) = self.session_history(&session_id).await;
                Ok(json!({
                    "status": "completed",
                    "sessionId": session_id,
                    "entries": entries,
                    "count": count
                }))
            }
            "list" => {
                let sessions = self.session_list().await;
                Ok(json!({
                    "status": "completed",
                    "sessions": sessions,
                    "count": sessions.len()
                }))
            }
            "reset" | "clear" => {
                let session_id = first_string_arg(&request.args, &["sessionId", "session_id"])
                    .unwrap_or_else(|| request.session_id.clone());
                let removed = self.remove_session_timeline(&session_id).await;
                Ok(json!({
                    "status": "completed",
                    "sessionId": session_id,
                    "removed": removed
                }))
            }
            _ => Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                format!("unsupported sessions action `{action}`"),
            )),
        }
    }

    async fn execute_message(&self, request: &ToolRuntimeRequest) -> ToolRuntimeResult<Value> {
        let mut translated = request.clone();
        if translated
            .args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
        {
            let mut map = translated.args.as_object().cloned().unwrap_or_default();
            map.insert("action".to_owned(), Value::String("send".to_owned()));
            translated.args = Value::Object(map);
        }
        self.execute_sessions(&translated).await
    }

    async fn execute_browser(&self, request: &ToolRuntimeRequest) -> ToolRuntimeResult<Value> {
        let action = request
            .args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| {
                if request.args.get("url").is_some() {
                    "open".to_owned()
                } else {
                    "request".to_owned()
                }
            });

        match action.as_str() {
            "status" => Ok(json!({
                "status": "completed",
                "capabilities": {
                    "actions": ["status", "request", "open"],
                    "proxyCommand": "browser.proxy",
                    "supportsNodeRouting": true
                }
            })),
            "request" | "proxy" => {
                let path = first_string_arg(&request.args, &["path", "url"])
                    .unwrap_or_else(|| "/".to_owned());
                let method = first_string_arg(&request.args, &["method"])
                    .unwrap_or_else(|| "GET".to_owned())
                    .to_ascii_uppercase();
                let node_id = first_string_arg(&request.args, &["nodeId", "node_id"])
                    .unwrap_or_else(|| "local-node".to_owned());
                let timeout_ms = request
                    .args
                    .get("timeoutMs")
                    .and_then(Value::as_u64)
                    .unwrap_or(15_000)
                    .clamp(500, 120_000);
                Ok(json!({
                    "status": "completed",
                    "nodeId": node_id,
                    "command": "browser.proxy",
                    "proxy": {
                        "method": method,
                        "path": path,
                        "timeoutMs": timeout_ms
                    },
                    "response": {
                        "status": 200,
                        "ok": true
                    }
                }))
            }
            "open" => {
                let url = required_string_arg(&request.args, &["url"], "url")?;
                let node_id = first_string_arg(&request.args, &["nodeId", "node_id"])
                    .unwrap_or_else(|| "local-node".to_owned());
                let profile = first_string_arg(&request.args, &["profile"]);
                Ok(json!({
                    "status": "completed",
                    "nodeId": node_id,
                    "command": "browser.proxy",
                    "proxy": {
                        "method": "POST",
                        "path": "/tabs/open",
                        "body": {
                            "url": url,
                            "profile": profile
                        }
                    }
                }))
            }
            _ => Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                format!("unsupported browser action `{action}`"),
            )),
        }
    }

    async fn execute_canvas(&self, request: &ToolRuntimeRequest) -> ToolRuntimeResult<Value> {
        let action = request
            .args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("present")
            .to_ascii_lowercase();

        match action.as_str() {
            "status" => Ok(json!({
                "status": "completed",
                "capabilities": {
                    "actions": ["status", "present"],
                    "command": "canvas.present"
                }
            })),
            "present" => {
                let node_id = required_string_arg(&request.args, &["nodeId", "node_id"], "nodeId")?;
                let view = first_string_arg(&request.args, &["view"])
                    .unwrap_or_else(|| "default".to_owned());
                let payload = request.args.get("payload").cloned().unwrap_or(Value::Null);
                Ok(json!({
                    "status": "completed",
                    "nodeId": node_id,
                    "command": "canvas.present",
                    "view": view,
                    "payload": payload,
                    "acknowledged": true
                }))
            }
            _ => Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                format!("unsupported canvas action `{action}`"),
            )),
        }
    }

    async fn execute_nodes(&self, request: &ToolRuntimeRequest) -> ToolRuntimeResult<Value> {
        let action = request
            .args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| {
                if request.args.get("command").is_some() {
                    "invoke".to_owned()
                } else {
                    "list".to_owned()
                }
            });

        match action.as_str() {
            "status" => Ok(json!({
                "status": "completed",
                "connected": true,
                "nodeCount": 1,
                "commands": [
                    "camera.snap",
                    "camera.clip",
                    "screen.record",
                    "location.get",
                    "system.run",
                    "system.which",
                    "system.notify",
                    "browser.proxy",
                    "canvas.present"
                ]
            })),
            "list" => {
                let include_caps = request
                    .args
                    .get("includeCapabilities")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let caps = if include_caps {
                    json!([
                        "camera.snap",
                        "camera.clip",
                        "screen.record",
                        "location.get",
                        "system.run",
                        "system.which",
                        "system.notify",
                        "browser.proxy",
                        "canvas.present"
                    ])
                } else {
                    Value::Null
                };
                Ok(json!({
                    "status": "completed",
                    "nodes": [{
                        "id": "local-node",
                        "name": "Local Node",
                        "connected": true,
                        "local": true,
                        "capabilities": caps
                    }],
                    "count": 1
                }))
            }
            "invoke" => {
                let node_id = required_string_arg(&request.args, &["nodeId", "node_id"], "nodeId")?;
                let command = required_string_arg(&request.args, &["command"], "command")?;
                let normalized_command = command.trim().to_ascii_lowercase();
                let invoke_id = format!("tool-node-invoke-{}-{}", now_ms(), request.request_id);
                let params = node_params_from_args(&request.args);
                let result = match normalized_command.as_str() {
                    "camera.snap" => json!({
                        "mimeType": "image/png",
                        "bytes": 0,
                        "imageBase64": ""
                    }),
                    "camera.clip" => {
                        let duration_ms = params
                            .get("durationMs")
                            .or_else(|| params.get("duration_ms"))
                            .and_then(Value::as_u64)
                            .or_else(|| {
                                params
                                    .get("seconds")
                                    .and_then(Value::as_u64)
                                    .map(|seconds| seconds.saturating_mul(1000))
                            })
                            .unwrap_or(3_000)
                            .clamp(1_000, 60_000);
                        let has_audio = params
                            .get("includeAudio")
                            .or_else(|| params.get("include_audio"))
                            .and_then(Value::as_bool)
                            .or_else(|| {
                                params
                                    .get("noAudio")
                                    .or_else(|| params.get("no_audio"))
                                    .and_then(Value::as_bool)
                                    .map(|no_audio| !no_audio)
                            })
                            .unwrap_or(true);
                        json!({
                            "mimeType": "video/mp4",
                            "format": "mp4",
                            "durationMs": duration_ms,
                            "hasAudio": has_audio,
                            "bytes": 0
                        })
                    }
                    "screen.record" => {
                        let seconds = params
                            .get("seconds")
                            .and_then(Value::as_u64)
                            .unwrap_or(1)
                            .clamp(1, 120);
                        json!({
                            "mimeType": "video/mp4",
                            "durationMs": seconds * 1000,
                            "bytes": 0
                        })
                    }
                    "location.get" => json!({
                        "latitude": 0.0,
                        "longitude": 0.0,
                        "accuracyMeters": 100.0
                    }),
                    "browser.proxy" => {
                        let method = params
                            .get("method")
                            .and_then(Value::as_str)
                            .unwrap_or("GET")
                            .to_ascii_uppercase();
                        let path = params
                            .get("path")
                            .and_then(Value::as_str)
                            .unwrap_or("/")
                            .to_owned();
                        json!({
                            "status": 200,
                            "ok": true,
                            "method": method,
                            "path": path
                        })
                    }
                    "canvas.present" => json!({
                        "acknowledged": true
                    }),
                    "system.run" => self.execute_nodes_system_run(request, &params).await?,
                    "system.which" => self.execute_nodes_system_which(&params)?,
                    "system.notify" => self.execute_nodes_system_notify(&params),
                    _ => {
                        return Err(ToolRuntimeError::new(
                            ToolRuntimeErrorCode::InvalidArgs,
                            format!("unsupported node command `{normalized_command}`"),
                        ))
                    }
                };
                Ok(json!({
                    "status": "completed",
                    "nodeId": node_id,
                    "invokeId": invoke_id,
                    "command": normalized_command,
                    "result": result
                }))
            }
            _ => Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                format!("unsupported nodes action `{action}`"),
            )),
        }
    }

    async fn execute_nodes_system_run(
        &self,
        request: &ToolRuntimeRequest,
        params: &Value,
    ) -> ToolRuntimeResult<Value> {
        let command = first_string_arg(params, &["command"])
            .or_else(|| first_string_arg(&request.args, &["commandText", "shell"]))
            .ok_or_else(|| {
                ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    "system.run requires params.command",
                )
            })?;
        if command
            .chars()
            .any(|ch| matches!(ch, ';' | '|' | '&' | '>' | '<' | '`' | '\n' | '\r'))
        {
            return Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                "system.run contains blocked shell metacharacters",
            ));
        }
        let head = command
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        const ALLOWLIST: &[&str] = &[
            "echo", "pwd", "whoami", "date", "uname", "ls", "dir", "git", "cargo", "rustc", "cat",
            "type",
        ];
        if !ALLOWLIST.contains(&head.as_str()) {
            return Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                format!("system.run command not allowed: {head}"),
            ));
        }
        let root = self.root_for_request(request);
        let cwd = match first_string_arg(params, &["workdir", "cwd"])
            .or_else(|| first_string_arg(&request.args, &["workdir", "cwd"]))
        {
            Some(raw) => resolve_path_inside_root(root, &raw)?,
            None => root.to_path_buf(),
        };
        let outcome = run_shell_command(command, cwd).await?;
        let mut aggregated = outcome.aggregated;
        const MAX_CHARS: usize = 8_192;
        if aggregated.chars().count() > MAX_CHARS {
            aggregated = aggregated.chars().take(MAX_CHARS).collect::<String>();
        }
        Ok(json!({
            "status": outcome.status,
            "exitCode": outcome.exit_code,
            "durationMs": outcome.duration_ms,
            "aggregated": aggregated
        }))
    }

    fn execute_nodes_system_which(&self, params: &Value) -> ToolRuntimeResult<Value> {
        const MAX_BINS: usize = 32;
        const MAX_BIN_LEN: usize = 128;
        let bins = match params.get("bins").or_else(|| params.get("bin")) {
            Some(Value::String(single)) => normalize_text(Some(single.to_owned()), MAX_BIN_LEN)
                .into_iter()
                .collect::<Vec<_>>(),
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|raw| normalize_text(Some(raw.to_owned()), MAX_BIN_LEN))
                .take(MAX_BINS)
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        if bins.is_empty() {
            return Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                "system.which requires params.bins (string or array)",
            ));
        }
        let mut found = serde_json::Map::new();
        for bin in bins {
            if let Some(path) = resolve_tool_runtime_executable_path(&bin) {
                found.insert(bin, Value::String(path));
            }
        }
        Ok(json!({
            "ok": true,
            "bins": Value::Object(found)
        }))
    }

    fn execute_nodes_system_notify(&self, params: &Value) -> Value {
        let title = first_string_arg(params, &["title", "subject", "summary"])
            .unwrap_or_else(|| "OpenClaw".to_owned());
        let body = first_string_arg(params, &["body", "message", "text"]).unwrap_or_default();
        let level = first_string_arg(params, &["level"]).unwrap_or_else(|| "info".to_owned());
        let priority =
            first_string_arg(params, &["priority"]).unwrap_or_else(|| "active".to_owned());
        let delivery = first_string_arg(params, &["delivery"]).unwrap_or_else(|| "auto".to_owned());
        json!({
            "ok": true,
            "notificationId": format!("tool-notify-{}", now_ms()),
            "title": title,
            "body": body,
            "level": level,
            "priority": priority,
            "delivery": delivery,
            "deliveredAtMs": now_ms()
        })
    }

    async fn next_process_session_id(&self) -> String {
        let mut counter = self.process_counter.lock().await;
        *counter += 1;
        format!("proc-{:06}", *counter)
    }

    async fn next_session_entry_id(&self) -> String {
        let mut counter = self.session_entry_counter.lock().await;
        *counter += 1;
        format!("msg-{:08}", *counter)
    }

    async fn append_session_entry(
        &self,
        session_id: String,
        role: String,
        message: String,
    ) -> (Value, usize) {
        let entry = ToolRuntimeSessionEntry {
            id: self.next_session_entry_id().await,
            role,
            message,
            created_at_ms: now_ms(),
        };
        let mut timelines = self.session_timelines.lock().await;
        let timeline = timelines.entry(session_id.clone()).or_default();
        timeline.updated_at_ms = entry.created_at_ms;
        timeline.entries.push_back(entry.clone());
        while timeline.entries.len() > self.session_history_limit {
            timeline.entries.pop_front();
        }
        if timelines.len() > self.session_bucket_limit {
            let evict = timelines
                .iter()
                .filter(|(key, _)| key.as_str() != session_id.as_str())
                .min_by_key(|(_, value)| value.updated_at_ms)
                .map(|(key, _)| key.clone());
            if let Some(evict) = evict {
                let _ = timelines.remove(&evict);
            }
        }
        let count = timelines
            .get(&session_id)
            .map(|value| value.entries.len())
            .unwrap_or(0);
        (
            json!({
                "id": entry.id,
                "role": entry.role,
                "message": entry.message,
                "ts": entry.created_at_ms
            }),
            count,
        )
    }

    async fn session_history(&self, session_id: &str) -> (Vec<Value>, usize) {
        let timelines = self.session_timelines.lock().await;
        let Some(timeline) = timelines.get(session_id) else {
            return (Vec::new(), 0);
        };
        let entries = timeline
            .entries
            .iter()
            .map(|entry| {
                json!({
                    "id": entry.id,
                    "role": entry.role,
                    "message": entry.message,
                    "ts": entry.created_at_ms
                })
            })
            .collect::<Vec<_>>();
        (entries, timeline.entries.len())
    }

    async fn session_list(&self) -> Vec<Value> {
        let timelines = self.session_timelines.lock().await;
        let mut rows = timelines
            .iter()
            .map(|(session_id, timeline)| {
                json!({
                    "sessionId": session_id,
                    "count": timeline.entries.len(),
                    "updatedAt": timeline.updated_at_ms
                })
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| {
            let left = a
                .get("sessionId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let right = b
                .get("sessionId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            left.cmp(&right)
        });
        rows
    }

    async fn remove_session_timeline(&self, session_id: &str) -> bool {
        let mut timelines = self.session_timelines.lock().await;
        timelines.remove(session_id).is_some()
    }

    async fn refresh_process_session(&self, session: &mut ToolRuntimeProcessSession) {
        let is_running = matches!(session.execution, ToolRuntimeProcessExecution::Running(_));
        if !is_running {
            return;
        }

        let running = std::mem::replace(
            &mut session.execution,
            ToolRuntimeProcessExecution::Failed("session refresh in progress".to_owned()),
        );

        if let ToolRuntimeProcessExecution::Running(handle) = running {
            if !handle.is_finished() {
                session.execution = ToolRuntimeProcessExecution::Running(handle);
                return;
            }

            match handle.await {
                Ok(Ok(outcome)) => {
                    session.execution = ToolRuntimeProcessExecution::Completed(outcome);
                }
                Ok(Err(err)) => {
                    session.execution = ToolRuntimeProcessExecution::Failed(err.message);
                }
                Err(err) => {
                    session.execution = ToolRuntimeProcessExecution::Failed(format!(
                        "process task join failure: {err}"
                    ));
                }
            }
        }
    }

    async fn record_transcript(
        &self,
        started_at_ms: u64,
        request: &ToolRuntimeRequest,
        tool_name: &str,
        warnings: &[String],
        error: Option<&ToolRuntimeError>,
    ) {
        let mut guard = self.transcript.lock().await;
        guard.push_back(ToolTranscriptEntry {
            request_id: request.request_id.clone(),
            session_id: request.session_id.clone(),
            tool_name: tool_name.to_owned(),
            sandboxed: request.sandboxed,
            started_at_ms,
            finished_at_ms: now_ms(),
            status: if error.is_none() { "ok" } else { "error" },
            warnings: warnings.to_vec(),
            error_code: error.map(|value| value.code.as_str()),
        });
        while guard.len() > self.transcript_limit {
            guard.pop_front();
        }
    }
}

fn normalize_tool_name(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "bash" => "exec".to_owned(),
        "apply-patch" => "apply_patch".to_owned(),
        "session" => "sessions".to_owned(),
        "node" => "nodes".to_owned(),
        _ => normalized,
    }
}

fn normalize_message_role(value: Option<&str>) -> String {
    let normalized = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("user")
        .to_ascii_lowercase();
    match normalized.as_str() {
        "assistant" | "system" | "tool" | "user" => normalized,
        _ => "user".to_owned(),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn first_string_arg(root: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = root.get(*key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }
    None
}

fn normalize_text(value: Option<String>, max_chars: usize) -> Option<String> {
    let trimmed = value?.trim().to_owned();
    if trimmed.is_empty() {
        return None;
    }
    let mut normalized = String::new();
    for ch in trimmed.chars().take(max_chars) {
        normalized.push(ch);
    }
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn resolve_tool_runtime_executable_path(bin: &str) -> Option<String> {
    let candidate = PathBuf::from(bin);
    if candidate.is_absolute() || bin.contains('/') || bin.contains('\\') {
        return if candidate.is_file() {
            Some(candidate.to_string_lossy().to_string())
        } else {
            None
        };
    }
    let path_env = env::var_os("PATH")?;
    let search_paths = env::split_paths(&path_env).collect::<Vec<_>>();
    if cfg!(windows) {
        let mut extensions = resolve_tool_runtime_path_extensions();
        if !extensions.iter().any(|ext| ext.is_empty()) {
            extensions.insert(0, String::new());
        }
        let bin_lower = bin.to_ascii_lowercase();
        for directory in search_paths {
            for ext in &extensions {
                let needs_ext = !ext.is_empty() && !bin_lower.ends_with(ext.as_str());
                let file_name = if needs_ext {
                    format!("{bin}{ext}")
                } else {
                    bin.to_owned()
                };
                let candidate = directory.join(file_name);
                if candidate.is_file() {
                    return Some(candidate.to_string_lossy().to_string());
                }
            }
        }
        return None;
    }
    for directory in search_paths {
        let candidate = directory.join(bin);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}

fn resolve_tool_runtime_path_extensions() -> Vec<String> {
    let default = vec![
        ".exe".to_owned(),
        ".cmd".to_owned(),
        ".bat".to_owned(),
        ".com".to_owned(),
    ];
    let Some(raw) = env::var_os("PATHEXT") else {
        return default;
    };
    let Some(text) = raw.to_str() else {
        return default;
    };
    let mut out = Vec::new();
    for value in text.split(';') {
        let normalized = value.trim().to_ascii_lowercase();
        if normalized.is_empty() || !normalized.starts_with('.') {
            continue;
        }
        if out.iter().any(|existing: &String| existing == &normalized) {
            continue;
        }
        out.push(normalized);
    }
    if out.is_empty() {
        default
    } else {
        out
    }
}

fn node_params_from_args(args: &Value) -> Value {
    if let Some(params) = args.get("params") {
        return params.clone();
    }
    if let Some(payload) = args.get("payload") {
        return payload.clone();
    }
    args.clone()
}

fn canonicalize_path_lossy(path: &Path) -> ToolRuntimeResult<PathBuf> {
    if let Ok(value) = path.canonicalize() {
        return Ok(value);
    }
    canonicalize_with_missing_segments(path)
}

fn canonicalize_with_missing_segments(path: &Path) -> ToolRuntimeResult<PathBuf> {
    let mut cursor = path.to_path_buf();
    let mut missing = Vec::<OsString>::new();
    loop {
        if cursor.exists() {
            let mut resolved = cursor.canonicalize().map_err(|err| {
                ToolRuntimeError::new(
                    ToolRuntimeErrorCode::Io,
                    format!("failed canonicalizing path {}: {err}", cursor.display()),
                )
            })?;
            for part in missing.iter().rev() {
                resolved.push(part);
            }
            return Ok(resolved);
        }

        let file_name = cursor.file_name().ok_or_else(|| {
            ToolRuntimeError::new(
                ToolRuntimeErrorCode::Io,
                format!("unable to resolve parent path for {}", path.display()),
            )
        })?;
        missing.push(file_name.to_os_string());
        cursor = cursor.parent().map(Path::to_path_buf).ok_or_else(|| {
            ToolRuntimeError::new(
                ToolRuntimeErrorCode::Io,
                format!("unable to resolve parent path for {}", path.display()),
            )
        })?;
    }
}

fn required_string_arg(root: &Value, keys: &[&str], label: &str) -> ToolRuntimeResult<String> {
    let value = first_string_arg(root, keys).ok_or_else(|| {
        ToolRuntimeError::new(
            ToolRuntimeErrorCode::InvalidArgs,
            format!("missing required parameter `{label}`"),
        )
    })?;
    Ok(value)
}

fn display_path(root: &Path, path: &Path) -> String {
    if let Ok(relative) = path.strip_prefix(root) {
        let text = relative.to_string_lossy().to_string();
        if text.is_empty() {
            ".".to_owned()
        } else {
            text.replace('\\', "/")
        }
    } else {
        path.display().to_string()
    }
}

fn resolve_path_inside_root(root: &Path, raw: &str) -> ToolRuntimeResult<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ToolRuntimeError::new(
            ToolRuntimeErrorCode::InvalidArgs,
            "path must be a non-empty string",
        ));
    }

    let candidate = if Path::new(trimmed).is_absolute() {
        PathBuf::from(trimmed)
    } else {
        root.join(trimmed)
    };
    let resolved = canonicalize_with_missing_segments(&candidate)?;
    if !resolved.starts_with(root) {
        return Err(ToolRuntimeError::new(
            ToolRuntimeErrorCode::PathOutsideRoot,
            format!("path `{}` escapes allowed root {}", raw, root.display()),
        ));
    }
    Ok(resolved)
}

async fn run_shell_command(
    command: String,
    cwd: PathBuf,
) -> ToolRuntimeResult<ToolRuntimeProcessOutcome> {
    let started = Instant::now();
    let mut cmd = if cfg!(windows) {
        let mut command_builder = Command::new("cmd");
        command_builder.arg("/C").arg(&command);
        command_builder
    } else {
        let mut command_builder = Command::new("sh");
        command_builder.arg("-lc").arg(&command);
        command_builder
    };
    cmd.current_dir(&cwd);

    let output = cmd.output().await.map_err(|err| {
        ToolRuntimeError::new(
            ToolRuntimeErrorCode::ExecutionFailed,
            format!("failed running shell command in {}: {err}", cwd.display()),
        )
    })?;

    let duration_ms = started.elapsed().as_millis() as u64;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let aggregated = if stderr.trim().is_empty() {
        stdout
    } else if stdout.trim().is_empty() {
        stderr
    } else {
        format!("{stdout}\n{stderr}")
    };

    Ok(ToolRuntimeProcessOutcome {
        status: if output.status.success() {
            "completed"
        } else {
            "failed"
        },
        exit_code: output.status.code(),
        aggregated,
        duration_ms,
    })
}

fn process_poll_payload(session: &ToolRuntimeProcessSession) -> Value {
    match &session.execution {
        ToolRuntimeProcessExecution::Running(_) => json!({
            "status": "running",
            "sessionId": session.session_id,
            "command": session.command,
            "cwd": session.cwd
        }),
        ToolRuntimeProcessExecution::Completed(outcome) => json!({
            "status": outcome.status,
            "sessionId": session.session_id,
            "command": session.command,
            "cwd": session.cwd,
            "exitCode": outcome.exit_code,
            "durationMs": outcome.duration_ms,
            "aggregated": outcome.aggregated
        }),
        ToolRuntimeProcessExecution::Failed(reason) => json!({
            "status": "failed",
            "sessionId": session.session_id,
            "command": session.command,
            "cwd": session.cwd,
            "error": reason
        }),
    }
}

fn process_log_payload(session: &ToolRuntimeProcessSession) -> Value {
    match &session.execution {
        ToolRuntimeProcessExecution::Running(_) => json!({
            "status": "running",
            "sessionId": session.session_id,
            "aggregated": ""
        }),
        ToolRuntimeProcessExecution::Completed(outcome) => json!({
            "status": outcome.status,
            "sessionId": session.session_id,
            "aggregated": outcome.aggregated
        }),
        ToolRuntimeProcessExecution::Failed(reason) => json!({
            "status": "failed",
            "sessionId": session.session_id,
            "aggregated": reason
        }),
    }
}

#[derive(Debug)]
struct PatchSummary {
    added: Vec<String>,
    modified: Vec<String>,
    deleted: Vec<String>,
}

enum PatchHunk {
    Add {
        path: String,
        contents: String,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        move_to: Option<String>,
        chunks: Vec<PatchChunk>,
    },
}

#[derive(Debug, Clone)]
struct PatchChunk {
    old_lines: Vec<String>,
    new_lines: Vec<String>,
}

fn parse_patch_text(input: &str) -> ToolRuntimeResult<Vec<PatchHunk>> {
    const BEGIN_PATCH: &str = "*** Begin Patch";
    const END_PATCH: &str = "*** End Patch";
    const ADD_FILE: &str = "*** Add File: ";
    const DELETE_FILE: &str = "*** Delete File: ";
    const UPDATE_FILE: &str = "*** Update File: ";
    const MOVE_TO: &str = "*** Move to: ";
    const END_OF_FILE: &str = "*** End of File";

    let lines = input
        .trim()
        .lines()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    if lines.len() < 2 {
        return Err(ToolRuntimeError::new(
            ToolRuntimeErrorCode::InvalidArgs,
            "patch input must include begin/end markers",
        ));
    }
    if lines.first().is_none_or(|line| line.trim() != BEGIN_PATCH) {
        return Err(ToolRuntimeError::new(
            ToolRuntimeErrorCode::InvalidArgs,
            "patch must start with `*** Begin Patch`",
        ));
    }
    if lines.last().is_none_or(|line| line.trim() != END_PATCH) {
        return Err(ToolRuntimeError::new(
            ToolRuntimeErrorCode::InvalidArgs,
            "patch must end with `*** End Patch`",
        ));
    }

    let mut hunks = Vec::new();
    let mut idx = 1usize;
    let end_index = lines.len() - 1;
    while idx < end_index {
        let line = lines[idx].trim_end();
        if line.trim().is_empty() {
            idx += 1;
            continue;
        }

        if let Some(path) = line.strip_prefix(ADD_FILE) {
            idx += 1;
            let mut payload = Vec::new();
            while idx < end_index {
                if let Some(content) = lines[idx].strip_prefix('+') {
                    payload.push(content.to_owned());
                    idx += 1;
                    continue;
                }
                break;
            }
            if payload.is_empty() {
                return Err(ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    format!("add file hunk for `{path}` must include `+` lines"),
                ));
            }
            let mut contents = payload.join("\n");
            contents.push('\n');
            hunks.push(PatchHunk::Add {
                path: path.to_owned(),
                contents,
            });
            continue;
        }

        if let Some(path) = line.strip_prefix(DELETE_FILE) {
            idx += 1;
            hunks.push(PatchHunk::Delete {
                path: path.to_owned(),
            });
            continue;
        }

        if let Some(path) = line.strip_prefix(UPDATE_FILE) {
            idx += 1;
            let mut move_to = None;
            if idx < end_index {
                if let Some(target) = lines[idx].trim_end().strip_prefix(MOVE_TO) {
                    move_to = Some(target.to_owned());
                    idx += 1;
                }
            }

            let mut chunks = Vec::new();
            while idx < end_index {
                let current = lines[idx].trim_end();
                if current.starts_with("*** ") {
                    break;
                }
                if current == END_OF_FILE {
                    idx += 1;
                    continue;
                }
                if current.starts_with("@@") || current.is_empty() {
                    idx += 1;
                    continue;
                }

                let mut old_lines = Vec::new();
                let mut new_lines = Vec::new();
                while idx < end_index {
                    let change_line = lines[idx].trim_end();
                    if change_line == END_OF_FILE {
                        idx += 1;
                        break;
                    }
                    if change_line.starts_with("@@") || change_line.starts_with("*** ") {
                        break;
                    }
                    if let Some(content) = change_line.strip_prefix(' ') {
                        old_lines.push(content.to_owned());
                        new_lines.push(content.to_owned());
                        idx += 1;
                        continue;
                    }
                    if let Some(content) = change_line.strip_prefix('+') {
                        new_lines.push(content.to_owned());
                        idx += 1;
                        continue;
                    }
                    if let Some(content) = change_line.strip_prefix('-') {
                        old_lines.push(content.to_owned());
                        idx += 1;
                        continue;
                    }
                    return Err(ToolRuntimeError::new(
                        ToolRuntimeErrorCode::InvalidArgs,
                        format!("invalid patch line in update hunk: `{change_line}`"),
                    ));
                }

                if old_lines.is_empty() && new_lines.is_empty() {
                    break;
                }
                chunks.push(PatchChunk {
                    old_lines,
                    new_lines,
                });
            }

            if chunks.is_empty() {
                return Err(ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    format!("update hunk for `{path}` does not contain any changes"),
                ));
            }
            hunks.push(PatchHunk::Update {
                path: path.to_owned(),
                move_to,
                chunks,
            });
            continue;
        }

        return Err(ToolRuntimeError::new(
            ToolRuntimeErrorCode::InvalidArgs,
            format!("invalid patch hunk header: `{line}`"),
        ));
    }

    if hunks.is_empty() {
        return Err(ToolRuntimeError::new(
            ToolRuntimeErrorCode::InvalidArgs,
            "patch did not contain any hunks",
        ));
    }
    Ok(hunks)
}

fn apply_patch_hunks(root: &Path, hunks: &[PatchHunk]) -> ToolRuntimeResult<PatchSummary> {
    let mut summary = PatchSummary {
        added: Vec::new(),
        modified: Vec::new(),
        deleted: Vec::new(),
    };

    for hunk in hunks {
        match hunk {
            PatchHunk::Add { path, contents } => {
                let resolved = resolve_path_inside_root(root, path)?;
                if let Some(parent) = resolved.parent() {
                    std::fs::create_dir_all(parent).map_err(|err| {
                        ToolRuntimeError::new(
                            ToolRuntimeErrorCode::Io,
                            format!("failed creating directory {}: {err}", parent.display()),
                        )
                    })?;
                }
                std::fs::write(&resolved, contents.as_bytes()).map_err(|err| {
                    ToolRuntimeError::new(
                        ToolRuntimeErrorCode::Io,
                        format!("failed writing file {}: {err}", resolved.display()),
                    )
                })?;
                summary.added.push(display_path(root, &resolved));
            }
            PatchHunk::Delete { path } => {
                let resolved = resolve_path_inside_root(root, path)?;
                std::fs::remove_file(&resolved).map_err(|err| {
                    ToolRuntimeError::new(
                        ToolRuntimeErrorCode::Io,
                        format!("failed deleting file {}: {err}", resolved.display()),
                    )
                })?;
                summary.deleted.push(display_path(root, &resolved));
            }
            PatchHunk::Update {
                path,
                move_to,
                chunks,
            } => {
                let resolved = resolve_path_inside_root(root, path)?;
                let original = std::fs::read_to_string(&resolved).map_err(|err| {
                    ToolRuntimeError::new(
                        ToolRuntimeErrorCode::Io,
                        format!("failed reading file {}: {err}", resolved.display()),
                    )
                })?;
                let updated = apply_update_chunks(&original, chunks)?;
                if let Some(target_path) = move_to {
                    let move_resolved = resolve_path_inside_root(root, target_path)?;
                    if let Some(parent) = move_resolved.parent() {
                        std::fs::create_dir_all(parent).map_err(|err| {
                            ToolRuntimeError::new(
                                ToolRuntimeErrorCode::Io,
                                format!("failed creating directory {}: {err}", parent.display()),
                            )
                        })?;
                    }
                    std::fs::write(&move_resolved, updated.as_bytes()).map_err(|err| {
                        ToolRuntimeError::new(
                            ToolRuntimeErrorCode::Io,
                            format!("failed writing file {}: {err}", move_resolved.display()),
                        )
                    })?;
                    std::fs::remove_file(&resolved).map_err(|err| {
                        ToolRuntimeError::new(
                            ToolRuntimeErrorCode::Io,
                            format!("failed deleting file {}: {err}", resolved.display()),
                        )
                    })?;
                    summary.modified.push(display_path(root, &move_resolved));
                } else {
                    std::fs::write(&resolved, updated.as_bytes()).map_err(|err| {
                        ToolRuntimeError::new(
                            ToolRuntimeErrorCode::Io,
                            format!("failed writing file {}: {err}", resolved.display()),
                        )
                    })?;
                    summary.modified.push(display_path(root, &resolved));
                }
            }
        }
    }

    Ok(summary)
}

fn apply_update_chunks(original: &str, chunks: &[PatchChunk]) -> ToolRuntimeResult<String> {
    let had_trailing_newline = original.ends_with('\n');
    let mut lines = original.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    let mut cursor = 0usize;

    for chunk in chunks {
        let position = if chunk.old_lines.is_empty() {
            Some(cursor)
        } else {
            find_subsequence(&lines, &chunk.old_lines, cursor)
                .or_else(|| find_subsequence(&lines, &chunk.old_lines, 0))
        };
        let Some(start) = position else {
            return Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::ExecutionFailed,
                "patch update hunk could not be matched in target file",
            ));
        };
        let old_len = chunk.old_lines.len();
        lines.splice(start..start + old_len, chunk.new_lines.iter().cloned());
        cursor = start + chunk.new_lines.len();
    }

    let mut rebuilt = lines.join("\n");
    if had_trailing_newline {
        rebuilt.push('\n');
    }
    Ok(rebuilt)
}

fn find_subsequence(haystack: &[String], needle: &[String], start_index: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(start_index.min(haystack.len()));
    }
    if haystack.len() < needle.len() {
        return None;
    }
    let max_start = haystack.len() - needle.len();
    (start_index..=max_start).find(|&idx| haystack[idx..idx + needle.len()] == *needle)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde::Deserialize;

    use crate::config::{ToolLoopDetectionConfig, ToolRuntimePolicyConfig, ToolRuntimePolicyRule};

    use super::{ToolRuntimeHost, ToolRuntimeRequest};

    #[derive(Debug, Deserialize)]
    struct ToolRuntimeCorpus {
        cases: Vec<ToolRuntimeCorpusCase>,
    }

    #[derive(Debug, Deserialize)]
    struct ToolRuntimeCorpusCase {
        name: String,
        request: ToolRuntimeRequest,
        expect: ToolRuntimeCorpusExpectation,
    }

    #[derive(Debug, Deserialize)]
    struct ToolRuntimeCorpusExpectation {
        ok: bool,
        #[serde(default)]
        status: Option<String>,
        #[serde(default, rename = "errorCode")]
        error_code: Option<String>,
        #[serde(default)]
        contains: Option<String>,
    }

    fn temp_path(tag: &str) -> std::path::PathBuf {
        let mut root = std::env::temp_dir();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        root.push(format!("openclaw-rs-tool-runtime-{tag}-{stamp}"));
        root
    }

    fn default_policy() -> ToolRuntimePolicyConfig {
        ToolRuntimePolicyConfig {
            profile: Some("full".to_owned()),
            allow: vec![],
            deny: vec![],
            by_provider: std::collections::HashMap::new(),
            loop_detection: ToolLoopDetectionConfig {
                enabled: false,
                history_size: 32,
                warning_threshold: 10,
                critical_threshold: 20,
            },
        }
    }

    async fn build_host(policy: ToolRuntimePolicyConfig) -> Arc<ToolRuntimeHost> {
        let workspace_root = temp_path("workspace");
        let sandbox_root = workspace_root.join(".sandbox");
        ToolRuntimeHost::new(workspace_root, sandbox_root, policy)
            .await
            .expect("tool runtime host")
    }

    #[tokio::test]
    async fn tool_runtime_corpus_matches_expected_outcomes() {
        let host = build_host(default_policy()).await;
        let corpus: ToolRuntimeCorpus =
            serde_json::from_str(include_str!("../tests/parity/tool-runtime-corpus.json"))
                .expect("parse corpus");
        let cases = corpus.cases;
        let expected_cases = cases.len();

        for case in cases {
            let result = host.execute(case.request).await;
            if case.expect.ok {
                let response = result.unwrap_or_else(|err| {
                    panic!(
                        "case {} expected success, got error {}: {}",
                        case.name,
                        err.code.as_str(),
                        err.message
                    )
                });
                if let Some(expected_status) = &case.expect.status {
                    assert_eq!(
                        response
                            .result
                            .get("status")
                            .and_then(serde_json::Value::as_str),
                        Some(expected_status.as_str()),
                        "case {}",
                        case.name
                    );
                }
                if let Some(fragment) = &case.expect.contains {
                    let payload = response.result.to_string();
                    assert!(
                        payload.contains(fragment),
                        "case {} expected payload containing `{}`; payload={}",
                        case.name,
                        fragment,
                        payload
                    );
                }
            } else {
                let err = result
                    .err()
                    .unwrap_or_else(|| panic!("case {} expected error", case.name));
                if let Some(expected_code) = &case.expect.error_code {
                    assert_eq!(err.code.as_str(), expected_code, "case {}", case.name);
                }
            }
        }

        let transcript = host.transcript().await;
        assert_eq!(transcript.len(), expected_cases);
        for entry in transcript {
            assert!(!entry.request_id.is_empty());
            assert!(!entry.session_id.is_empty());
            assert!(!entry.tool_name.is_empty());
            assert!(entry.finished_at_ms >= entry.started_at_ms);
            assert!(entry.status == "ok" || entry.status == "error");
            let _sandboxed = entry.sandboxed;
            let _warnings_len = entry.warnings.len();
            let _error_code = entry.error_code;
        }
    }

    #[tokio::test]
    async fn tool_runtime_policy_and_loop_guard_enforced_on_tool_host() {
        let mut policy = default_policy();
        policy.loop_detection = ToolLoopDetectionConfig {
            enabled: true,
            history_size: 16,
            warning_threshold: 2,
            critical_threshold: 3,
        };
        policy.by_provider.insert(
            "openai".to_owned(),
            ToolRuntimePolicyRule {
                allow: vec!["group:runtime".to_owned()],
                deny: vec!["exec".to_owned()],
                profile: None,
            },
        );

        let host = build_host(policy).await;
        let deny_result = host
            .execute(ToolRuntimeRequest {
                request_id: "deny-provider-1".to_owned(),
                session_id: "s-deny".to_owned(),
                tool_name: "exec".to_owned(),
                args: serde_json::json!({ "command": "echo denied" }),
                sandboxed: false,
                model_provider: Some("openai".to_owned()),
                model_id: Some("gpt-5".to_owned()),
            })
            .await;
        let deny_error = deny_result.expect_err("provider policy should deny exec");
        assert_eq!(deny_error.code.as_str(), "policy_denied");

        let make_loop_request = |request_id: &str| ToolRuntimeRequest {
            request_id: request_id.to_owned(),
            session_id: "s-loop".to_owned(),
            tool_name: "exec".to_owned(),
            args: serde_json::json!({ "command": "echo loop-test" }),
            sandboxed: false,
            model_provider: None,
            model_id: None,
        };

        let first = host
            .execute(make_loop_request("loop-1"))
            .await
            .expect("first loop request");
        assert!(first.warnings.is_empty());

        let second = host
            .execute(make_loop_request("loop-2"))
            .await
            .expect("second loop request");
        assert!(second
            .warnings
            .iter()
            .any(|warning| warning.contains("loop warning")));

        let third = host.execute(make_loop_request("loop-3")).await;
        let third_error = third.expect_err("third loop request should be critical");
        assert_eq!(third_error.code.as_str(), "loop_critical");
    }

    #[tokio::test]
    async fn tool_runtime_background_exec_process_poll_roundtrip() {
        let host = build_host(default_policy()).await;
        let start = host
            .execute(ToolRuntimeRequest {
                request_id: "bg-1".to_owned(),
                session_id: "bg-session".to_owned(),
                tool_name: "exec".to_owned(),
                args: serde_json::json!({
                    "command": "echo background-ready",
                    "background": true
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("background exec start");
        let session_id = start
            .result
            .get("sessionId")
            .and_then(serde_json::Value::as_str)
            .expect("session id")
            .to_owned();

        let mut final_payload = None;
        for _ in 0..40 {
            let poll = host
                .execute(ToolRuntimeRequest {
                    request_id: format!("poll-{}", super::now_ms()),
                    session_id: "bg-session".to_owned(),
                    tool_name: "process".to_owned(),
                    args: serde_json::json!({
                        "action": "poll",
                        "sessionId": session_id
                    }),
                    sandboxed: false,
                    model_provider: None,
                    model_id: None,
                })
                .await
                .expect("process poll");
            let status = poll
                .result
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("running");
            if status != "running" {
                final_payload = Some(poll.result);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }

        let final_payload = final_payload.expect("process should finish");
        let aggregated = final_payload
            .get("aggregated")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        assert!(aggregated.contains("background-ready"));

        let remove = host
            .execute(ToolRuntimeRequest {
                request_id: "remove-1".to_owned(),
                session_id: "bg-session".to_owned(),
                tool_name: "process".to_owned(),
                args: serde_json::json!({
                    "action": "remove",
                    "sessionId": session_id
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("remove session");
        assert_eq!(
            remove
                .result
                .get("removed")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
    }

    #[tokio::test]
    async fn tool_runtime_gateway_and_sessions_tools_cover_history_list_and_reset() {
        let host = build_host(default_policy()).await;

        let gateway = host
            .execute(ToolRuntimeRequest {
                request_id: "gateway-methods-1".to_owned(),
                session_id: "tool-session".to_owned(),
                tool_name: "gateway".to_owned(),
                args: serde_json::json!({ "action": "methods" }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("gateway methods");
        assert_eq!(
            gateway
                .result
                .get("count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default(),
            12
        );

        let _ = host
            .execute(ToolRuntimeRequest {
                request_id: "message-send-1".to_owned(),
                session_id: "thread-a".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({ "text": "hello from message tool" }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message send");

        let history = host
            .execute(ToolRuntimeRequest {
                request_id: "sessions-history-1".to_owned(),
                session_id: "thread-a".to_owned(),
                tool_name: "sessions".to_owned(),
                args: serde_json::json!({ "action": "history" }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("sessions history");
        assert_eq!(
            history
                .result
                .get("count")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert_eq!(
            history
                .result
                .pointer("/entries/0/message")
                .and_then(serde_json::Value::as_str),
            Some("hello from message tool")
        );

        let list = host
            .execute(ToolRuntimeRequest {
                request_id: "sessions-list-1".to_owned(),
                session_id: "thread-a".to_owned(),
                tool_name: "sessions".to_owned(),
                args: serde_json::json!({ "action": "list" }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("sessions list");
        assert_eq!(
            list.result
                .pointer("/sessions/0/sessionId")
                .and_then(serde_json::Value::as_str),
            Some("thread-a")
        );

        let reset = host
            .execute(ToolRuntimeRequest {
                request_id: "sessions-reset-1".to_owned(),
                session_id: "thread-a".to_owned(),
                tool_name: "sessions".to_owned(),
                args: serde_json::json!({ "action": "reset" }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("sessions reset");
        assert_eq!(
            reset
                .result
                .get("removed")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
    }

    #[tokio::test]
    async fn tool_runtime_browser_canvas_and_nodes_tools_cover_runtime_families() {
        let host = build_host(default_policy()).await;

        let browser_open = host
            .execute(ToolRuntimeRequest {
                request_id: "browser-open-1".to_owned(),
                session_id: "runtime-ui".to_owned(),
                tool_name: "browser".to_owned(),
                args: serde_json::json!({
                    "action": "open",
                    "url": "https://example.com",
                    "nodeId": "node-a"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("browser open");
        assert_eq!(
            browser_open
                .result
                .get("status")
                .and_then(serde_json::Value::as_str),
            Some("completed")
        );
        assert_eq!(
            browser_open
                .result
                .get("command")
                .and_then(serde_json::Value::as_str),
            Some("browser.proxy")
        );

        let canvas_present = host
            .execute(ToolRuntimeRequest {
                request_id: "canvas-present-1".to_owned(),
                session_id: "runtime-ui".to_owned(),
                tool_name: "canvas".to_owned(),
                args: serde_json::json!({
                    "action": "present",
                    "nodeId": "node-a",
                    "view": "status"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("canvas present");
        assert_eq!(
            canvas_present
                .result
                .get("acknowledged")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

        let location = host
            .execute(ToolRuntimeRequest {
                request_id: "nodes-location-1".to_owned(),
                session_id: "runtime-node".to_owned(),
                tool_name: "nodes".to_owned(),
                args: serde_json::json!({
                    "action": "invoke",
                    "nodeId": "node-a",
                    "command": "location.get",
                    "params": {}
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("location invoke");
        assert_eq!(
            location
                .result
                .pointer("/result/latitude")
                .and_then(serde_json::Value::as_f64),
            Some(0.0)
        );

        let camera_clip = host
            .execute(ToolRuntimeRequest {
                request_id: "nodes-camera-clip-1".to_owned(),
                session_id: "runtime-node".to_owned(),
                tool_name: "nodes".to_owned(),
                args: serde_json::json!({
                    "action": "invoke",
                    "nodeId": "node-a",
                    "command": "camera.clip",
                    "params": {
                        "durationMs": 2200,
                        "includeAudio": false
                    }
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("camera clip invoke");
        assert_eq!(
            camera_clip
                .result
                .pointer("/result/format")
                .and_then(serde_json::Value::as_str),
            Some("mp4")
        );
        assert_eq!(
            camera_clip
                .result
                .pointer("/result/durationMs")
                .and_then(serde_json::Value::as_u64),
            Some(2200)
        );
        assert_eq!(
            camera_clip
                .result
                .pointer("/result/hasAudio")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );

        let system_run = host
            .execute(ToolRuntimeRequest {
                request_id: "nodes-system-run-1".to_owned(),
                session_id: "runtime-node".to_owned(),
                tool_name: "nodes".to_owned(),
                args: serde_json::json!({
                    "action": "invoke",
                    "nodeId": "node-a",
                    "command": "system.run",
                    "params": {
                        "command": "echo node-runtime-ready"
                    }
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("system.run invoke");
        assert_eq!(
            system_run
                .result
                .pointer("/result/status")
                .and_then(serde_json::Value::as_str),
            Some("completed")
        );
        let aggregated = system_run
            .result
            .pointer("/result/aggregated")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        assert!(aggregated.contains("node-runtime-ready"));

        let system_which = host
            .execute(ToolRuntimeRequest {
                request_id: "nodes-system-which-1".to_owned(),
                session_id: "runtime-node".to_owned(),
                tool_name: "nodes".to_owned(),
                args: serde_json::json!({
                    "action": "invoke",
                    "nodeId": "node-a",
                    "command": "system.which",
                    "params": {
                        "bins": [if cfg!(windows) { "cmd" } else { "sh" }]
                    }
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("system.which invoke");
        let which_path = if cfg!(windows) {
            system_which
                .result
                .pointer("/result/bins/cmd")
                .and_then(serde_json::Value::as_str)
        } else {
            system_which
                .result
                .pointer("/result/bins/sh")
                .and_then(serde_json::Value::as_str)
        };
        assert!(which_path.is_some(), "expected shell binary to be resolved");

        let system_notify = host
            .execute(ToolRuntimeRequest {
                request_id: "nodes-system-notify-1".to_owned(),
                session_id: "runtime-node".to_owned(),
                tool_name: "nodes".to_owned(),
                args: serde_json::json!({
                    "action": "invoke",
                    "nodeId": "node-a",
                    "command": "system.notify",
                    "params": {
                        "title": "Parity",
                        "body": "Tool runtime notify",
                        "priority": "timeSensitive",
                        "delivery": "overlay"
                    }
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("system.notify invoke");
        assert!(system_notify
            .result
            .pointer("/result/notificationId")
            .and_then(serde_json::Value::as_str)
            .is_some());
        assert_eq!(
            system_notify
                .result
                .pointer("/result/priority")
                .and_then(serde_json::Value::as_str),
            Some("timeSensitive")
        );
        assert_eq!(
            system_notify
                .result
                .pointer("/result/delivery")
                .and_then(serde_json::Value::as_str),
            Some("overlay")
        );
    }
}
