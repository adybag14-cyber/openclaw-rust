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

use crate::channels::{normalize_channel_id, ChannelCapabilities, DriverRegistry};
use crate::config::ToolRuntimePolicyConfig;
use crate::security::tool_loop::{ToolLoopGuard, ToolLoopLevel};
use crate::security::tool_policy::ToolPolicyMatcher;
use crate::session_key::parse_session_key;
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
    thread_id: Option<String>,
    created_at_ms: u64,
    edited_at_ms: Option<u64>,
    deleted_at_ms: Option<u64>,
    pinned_at_ms: Option<u64>,
    reactions: Vec<ToolRuntimeSessionReaction>,
}

#[derive(Debug, Clone)]
struct ToolRuntimeSessionReaction {
    emoji: String,
    actor: String,
    created_at_ms: u64,
}

#[derive(Debug, Clone, Default)]
struct ToolRuntimeSessionTimeline {
    entries: VecDeque<ToolRuntimeSessionEntry>,
    updated_at_ms: u64,
}

#[derive(Debug, Clone)]
struct ToolRuntimeMessageThread {
    id: String,
    name: String,
    created_at_ms: u64,
    source_message_id: Option<String>,
    archived: bool,
}

#[derive(Debug, Clone, Default)]
struct ToolRuntimeThreadRegistry {
    threads: VecDeque<ToolRuntimeMessageThread>,
    updated_at_ms: u64,
}

#[derive(Debug, Clone)]
struct ToolRuntimeMessageEvent {
    id: String,
    guild_id: String,
    name: String,
    start_time: String,
    end_time: Option<String>,
    description: Option<String>,
    channel_id: Option<String>,
    location: Option<String>,
    event_type: String,
    created_at_ms: u64,
}

#[derive(Debug, Clone)]
struct ToolRuntimeMessageEmoji {
    id: String,
    guild_id: Option<String>,
    name: String,
    media: Option<String>,
    created_at_ms: u64,
}

#[derive(Debug, Clone)]
struct ToolRuntimeMessageSticker {
    id: String,
    guild_id: String,
    name: String,
    description: String,
    tags: String,
    media: Option<String>,
    created_at_ms: u64,
}

const TOOL_RUNTIME_NODE_COMMANDS: &[&str] = &[
    "camera.list",
    "camera.snap",
    "camera.clip",
    "screen.record",
    "location.get",
    "device.info",
    "device.status",
    "contacts.search",
    "calendar.events",
    "reminders.list",
    "photos.latest",
    "motion.activity",
    "motion.pedometer",
    "system.run",
    "system.which",
    "system.notify",
    "browser.proxy",
    "canvas.hide",
    "canvas.navigate",
    "canvas.eval",
    "canvas.snapshot",
    "canvas.a2ui.push",
    "canvas.a2ui.pushJSONL",
    "canvas.a2ui.reset",
    "canvas.present",
];

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
    thread_counter: Mutex<u64>,
    message_event_counter: Mutex<u64>,
    emoji_counter: Mutex<u64>,
    sticker_counter: Mutex<u64>,
    process_sessions: Mutex<HashMap<String, ToolRuntimeProcessSession>>,
    session_timelines: Mutex<HashMap<String, ToolRuntimeSessionTimeline>>,
    session_threads: Mutex<HashMap<String, ToolRuntimeThreadRegistry>>,
    message_events: Mutex<HashMap<String, VecDeque<ToolRuntimeMessageEvent>>>,
    message_emojis: Mutex<HashMap<String, VecDeque<ToolRuntimeMessageEmoji>>>,
    message_stickers: Mutex<HashMap<String, VecDeque<ToolRuntimeMessageSticker>>>,
    member_roles: Mutex<HashMap<String, Vec<String>>>,
    message_channel_capabilities: HashMap<String, ChannelCapabilities>,
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
        let message_channel_capabilities = DriverRegistry::default_registry()
            .capabilities()
            .into_iter()
            .map(|capability| (capability.name.to_owned(), capability))
            .collect::<HashMap<_, _>>();

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
            thread_counter: Mutex::new(0),
            message_event_counter: Mutex::new(0),
            emoji_counter: Mutex::new(0),
            sticker_counter: Mutex::new(0),
            process_sessions: Mutex::new(HashMap::new()),
            session_timelines: Mutex::new(HashMap::new()),
            session_threads: Mutex::new(HashMap::new()),
            message_events: Mutex::new(HashMap::new()),
            message_emojis: Mutex::new(HashMap::new()),
            message_stickers: Mutex::new(HashMap::new()),
            member_roles: Mutex::new(HashMap::new()),
            message_channel_capabilities,
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
                    .append_session_entry(session_id.clone(), role, message, None)
                    .await;
                let message_id = entry
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                Ok(json!({
                    "status": "completed",
                    "sessionId": session_id,
                    "messageId": message_id,
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
        let action = request
            .args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| {
                if request.args.get("pollQuestion").is_some()
                    || request.args.get("poll_question").is_some()
                    || request.args.get("pollOptions").is_some()
                    || request.args.get("poll_options").is_some()
                {
                    "poll".to_owned()
                } else {
                    "send".to_owned()
                }
            });

        match action.as_str() {
            "send" | "append" => self.execute_message_send(request).await,
            "poll" => self.execute_message_poll(request).await,
            "read" => self.execute_message_read(request).await,
            "edit" => self.execute_message_edit(request).await,
            "delete" | "remove" => self.execute_message_delete(request).await,
            "react" | "reaction" => self.execute_message_react(request).await,
            "reactions" => self.execute_message_reactions(request).await,
            "pin" => self.execute_message_pin(request).await,
            "unpin" => self.execute_message_unpin(request).await,
            "pins" | "list-pins" | "list_pins" => self.execute_message_list_pins(request).await,
            "permissions" => self.execute_message_permissions(request).await,
            "search" => self.execute_message_search(request).await,
            "thread-create" | "thread_create" => self.execute_message_thread_create(request).await,
            "thread-list" | "thread_list" => self.execute_message_thread_list(request).await,
            "thread-reply" | "thread_reply" => self.execute_message_thread_reply(request).await,
            "member-info" | "member_info" => self.execute_message_member_info(request).await,
            "role-info" | "role_info" => self.execute_message_role_info(request).await,
            "channel-info" | "channel_info" => self.execute_message_channel_info(request).await,
            "channel-list" | "channel_list" => self.execute_message_channel_list(request).await,
            "voice-status" | "voice_status" => self.execute_message_voice_status(request).await,
            "event-list" | "event_list" => self.execute_message_event_list(request).await,
            "event-create" | "event_create" => self.execute_message_event_create(request).await,
            "emoji-list" | "emoji_list" => self.execute_message_emoji_list(request).await,
            "emoji-upload" | "emoji_upload" => self.execute_message_emoji_upload(request).await,
            "sticker-send" | "sticker_send" => self.execute_message_sticker_send(request).await,
            "sticker-upload" | "sticker_upload" => {
                self.execute_message_sticker_upload(request).await
            }
            "role-add" | "role_add" => {
                self.execute_message_role_mutation(request, "role-add")
                    .await
            }
            "role-remove" | "role_remove" => {
                self.execute_message_role_mutation(request, "role-remove")
                    .await
            }
            "timeout" => self.execute_message_timeout(request).await,
            "kick" => self.execute_message_kick(request).await,
            "ban" => self.execute_message_ban(request).await,
            "history" | "list" | "reset" | "clear" => {
                let mut translated = request.clone();
                let mut map = translated.args.as_object().cloned().unwrap_or_default();
                map.insert("action".to_owned(), Value::String(action));
                translated.args = Value::Object(map);
                self.execute_sessions(&translated).await
            }
            _ => Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                format!("unsupported message action `{action}`"),
            )),
        }
    }

    fn resolve_message_channel_for_capability(
        &self,
        request: &ToolRuntimeRequest,
    ) -> Option<(String, bool)> {
        if let Some(raw_channel) = first_string_arg(&request.args, &["channel"]) {
            let normalized = normalize_channel_id(Some(raw_channel.as_str()))
                .unwrap_or_else(|| raw_channel.trim().to_ascii_lowercase());
            return Some((normalized, true));
        }

        let session_id = resolve_message_session_id(request);
        let parsed = parse_session_key(&session_id);
        parsed.channel.map(|channel| {
            let normalized = normalize_channel_id(Some(channel.as_str()))
                .unwrap_or_else(|| channel.trim().to_ascii_lowercase());
            (normalized, false)
        })
    }

    fn message_channel_supports_action(capability: &ChannelCapabilities, action: &str) -> bool {
        match action {
            "poll" => matches!(
                capability.name,
                "whatsapp" | "telegram" | "discord" | "matrix" | "msteams"
            ),
            "react" => matches!(
                capability.name,
                "discord" | "googlechat" | "slack" | "telegram" | "whatsapp" | "signal"
            ),
            "reactions" => matches!(capability.name, "discord" | "googlechat" | "slack"),
            "read" => matches!(capability.name, "discord" | "slack"),
            "edit" => matches!(capability.name, "discord" | "slack"),
            "delete" => matches!(capability.name, "discord" | "slack" | "telegram"),
            "pin" | "unpin" | "pins" => matches!(capability.name, "discord" | "slack"),
            "permissions" | "search" => capability.name == "discord",
            "thread-create" | "thread-list" | "thread-reply" => capability.name == "discord",
            "emoji-list" => matches!(capability.name, "discord" | "slack"),
            "emoji-upload" | "sticker-send" | "sticker-upload" => capability.name == "discord",
            "member-info" => matches!(capability.name, "discord" | "slack"),
            "role-info" | "channel-info" | "channel-list" | "voice-status" | "event-list"
            | "event-create" | "role-add" | "role-remove" | "timeout" | "kick" | "ban" => {
                capability.name == "discord"
            }
            _ => true,
        }
    }

    fn enforce_message_channel_action_support(
        &self,
        request: &ToolRuntimeRequest,
        action: &str,
    ) -> ToolRuntimeResult<Option<String>> {
        let Some((channel, explicit)) = self.resolve_message_channel_for_capability(request) else {
            return Ok(None);
        };
        let Some(capability) = self.message_channel_capabilities.get(channel.as_str()) else {
            if explicit {
                return Err(ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    format!("unsupported channel: {channel}"),
                ));
            }
            return Ok(None);
        };
        if !Self::message_channel_supports_action(capability, action) {
            return Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                format!("unsupported {action} channel: {}", capability.name),
            ));
        }
        Ok(Some(capability.name.to_owned()))
    }

    async fn execute_message_send(&self, request: &ToolRuntimeRequest) -> ToolRuntimeResult<Value> {
        let message = first_string_arg(&request.args, &["message", "content", "text", "prompt"])
            .ok_or_else(|| {
                ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    "missing required parameter `message`",
                )
            })?;
        let session_id = resolve_message_session_id(request);
        let thread_id = first_string_arg(&request.args, &["threadId", "thread_id", "thread"]);
        let role =
            normalize_message_role(first_string_arg(&request.args, &["role", "author"]).as_deref());
        let (entry, count) = self
            .append_session_entry(session_id.clone(), role, message, thread_id.clone())
            .await;
        let message_id = entry
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Ok(json!({
            "status": "completed",
            "action": "send",
            "sessionId": session_id,
            "messageId": message_id,
            "threadId": thread_id,
            "entry": entry,
            "count": count
        }))
    }

    async fn execute_message_poll(&self, request: &ToolRuntimeRequest) -> ToolRuntimeResult<Value> {
        let question = first_string_arg(
            &request.args,
            &["question", "pollQuestion", "poll_question", "title"],
        )
        .ok_or_else(|| {
            ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                "missing required parameter `question`",
            )
        })?;
        let options = first_string_list_arg(
            &request.args,
            &[
                "options",
                "pollOptions",
                "poll_options",
                "pollOption",
                "poll_option",
            ],
            12,
            256,
        );
        if options.len() < 2 {
            return Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                "poll requires at least two options",
            ));
        }
        let multi =
            first_bool_arg(&request.args, &["multi", "pollMulti", "poll_multi"]).unwrap_or(false);
        let anonymous = first_bool_arg(
            &request.args,
            &["anonymous", "pollAnonymous", "poll_anonymous"],
        )
        .unwrap_or(false);
        let duration_seconds = request
            .args
            .get("durationSeconds")
            .or_else(|| request.args.get("pollDurationSeconds"))
            .or_else(|| request.args.get("poll_duration_seconds"))
            .and_then(Value::as_u64)
            .map(|value| value.clamp(5, 86_400));
        let session_id = resolve_message_session_id(request);
        let thread_id = first_string_arg(&request.args, &["threadId", "thread_id", "thread"]);
        let _channel = self.enforce_message_channel_action_support(request, "poll")?;
        let (entry, count) = self
            .append_session_entry(
                session_id.clone(),
                "user".to_owned(),
                format!("[poll] {question}"),
                thread_id.clone(),
            )
            .await;
        let message_id = entry
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Ok(json!({
            "status": "completed",
            "action": "poll",
            "sessionId": session_id,
            "messageId": message_id,
            "threadId": thread_id,
            "entry": entry,
            "count": count,
            "poll": {
                "id": format!("poll-{}", now_ms()),
                "question": question,
                "options": options,
                "multi": multi,
                "anonymous": anonymous,
                "durationSeconds": duration_seconds
            }
        }))
    }

    async fn execute_message_read(&self, request: &ToolRuntimeRequest) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "read")?;
        let limit = request
            .args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(50)
            .clamp(1, 200) as usize;
        let include_deleted =
            first_bool_arg(&request.args, &["includeDeleted", "include_deleted"]).unwrap_or(false);
        let before_id = first_string_arg(&request.args, &["before", "beforeId", "before_id"]);
        let after_id = first_string_arg(&request.args, &["after", "afterId", "after_id"]);
        let thread_id_filter =
            first_string_arg(&request.args, &["threadId", "thread_id", "thread"]);

        let timelines = self.session_timelines.lock().await;
        let Some(timeline) = timelines.get(&session_id) else {
            return Ok(json!({
                "status": "completed",
                "action": "read",
                "sessionId": session_id,
                "messages": [],
                "count": 0,
                "hasMore": false
            }));
        };

        let entries = timeline.entries.iter().collect::<Vec<_>>();
        let mut start = 0usize;
        let mut end = entries.len();
        if let Some(after_id) = after_id.as_deref() {
            if let Some(pos) = entries.iter().position(|entry| entry.id == after_id) {
                start = pos.saturating_add(1);
            }
        }
        if let Some(before_id) = before_id.as_deref() {
            if let Some(pos) = entries.iter().position(|entry| entry.id == before_id) {
                end = end.min(pos);
            }
        }
        if start > end {
            start = end;
        }
        let mut selected = entries[start..end]
            .iter()
            .copied()
            .filter(|entry| {
                (include_deleted || entry.deleted_at_ms.is_none())
                    && match thread_id_filter.as_deref() {
                        Some(filter) => entry.thread_id.as_deref() == Some(filter),
                        None => true,
                    }
            })
            .collect::<Vec<_>>();
        let has_more = selected.len() > limit;
        if has_more {
            selected = selected.split_off(selected.len() - limit);
        }
        let messages = selected
            .into_iter()
            .map(serialize_session_entry)
            .collect::<Vec<_>>();
        Ok(json!({
            "status": "completed",
            "action": "read",
            "sessionId": session_id,
            "messages": messages,
            "count": messages.len(),
            "hasMore": has_more,
            "limit": limit,
            "threadId": thread_id_filter
        }))
    }

    async fn execute_message_edit(&self, request: &ToolRuntimeRequest) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "edit")?;
        let message = first_string_arg(&request.args, &["message", "content", "text", "prompt"])
            .ok_or_else(|| {
                ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    "missing required parameter `message`",
                )
            })?;
        let explicit_message_id =
            first_string_arg(&request.args, &["messageId", "message_id", "id"]);

        let mut timelines = self.session_timelines.lock().await;
        let timeline = timelines.get_mut(&session_id).ok_or_else(|| {
            ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                format!("session not found: {session_id}"),
            )
        })?;
        let message_id =
            resolve_target_message_id(timeline, explicit_message_id).ok_or_else(|| {
                ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    "missing required parameter `messageId`",
                )
            })?;
        let now = now_ms();
        let entry = timeline
            .entries
            .iter_mut()
            .find(|entry| entry.id == message_id)
            .ok_or_else(|| {
                ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    format!("message not found: {message_id}"),
                )
            })?;
        if entry.deleted_at_ms.is_some() {
            return Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                "cannot edit a deleted message",
            ));
        }
        entry.message = message;
        entry.edited_at_ms = Some(now);
        timeline.updated_at_ms = now;
        Ok(json!({
            "status": "completed",
            "action": "edit",
            "sessionId": session_id,
            "messageId": message_id,
            "entry": serialize_session_entry(entry),
            "edited": true
        }))
    }

    async fn execute_message_delete(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "delete")?;
        let explicit_message_id =
            first_string_arg(&request.args, &["messageId", "message_id", "id"]);

        let mut timelines = self.session_timelines.lock().await;
        let timeline = timelines.get_mut(&session_id).ok_or_else(|| {
            ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                format!("session not found: {session_id}"),
            )
        })?;
        let message_id =
            resolve_target_message_id(timeline, explicit_message_id).ok_or_else(|| {
                ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    "missing required parameter `messageId`",
                )
            })?;
        let now = now_ms();
        let entry = timeline
            .entries
            .iter_mut()
            .find(|entry| entry.id == message_id)
            .ok_or_else(|| {
                ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    format!("message not found: {message_id}"),
                )
            })?;
        let deleted = if entry.deleted_at_ms.is_some() {
            false
        } else {
            entry.deleted_at_ms = Some(now);
            entry.message = "[deleted]".to_owned();
            entry.pinned_at_ms = None;
            entry.reactions.clear();
            true
        };
        timeline.updated_at_ms = now;
        Ok(json!({
            "status": "completed",
            "action": "delete",
            "sessionId": session_id,
            "messageId": message_id,
            "deleted": deleted,
            "entry": serialize_session_entry(entry)
        }))
    }

    async fn execute_message_pin(&self, request: &ToolRuntimeRequest) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "pin")?;
        let explicit_message_id =
            first_string_arg(&request.args, &["messageId", "message_id", "id"]);
        let mut timelines = self.session_timelines.lock().await;
        let timeline = timelines.get_mut(&session_id).ok_or_else(|| {
            ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                format!("session not found: {session_id}"),
            )
        })?;
        let message_id =
            resolve_target_message_id(timeline, explicit_message_id).ok_or_else(|| {
                ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    "missing required parameter `messageId`",
                )
            })?;
        let now = now_ms();
        let entry = timeline
            .entries
            .iter_mut()
            .find(|entry| entry.id == message_id)
            .ok_or_else(|| {
                ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    format!("message not found: {message_id}"),
                )
            })?;
        if entry.deleted_at_ms.is_some() {
            return Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                "cannot pin a deleted message",
            ));
        }
        let pinned = if entry.pinned_at_ms.is_some() {
            false
        } else {
            entry.pinned_at_ms = Some(now);
            true
        };
        timeline.updated_at_ms = now;
        Ok(json!({
            "status": "completed",
            "action": "pin",
            "sessionId": session_id,
            "messageId": message_id,
            "pinned": pinned,
            "entry": serialize_session_entry(entry)
        }))
    }

    async fn execute_message_unpin(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "unpin")?;
        let explicit_message_id =
            first_string_arg(&request.args, &["messageId", "message_id", "id"]);
        let mut timelines = self.session_timelines.lock().await;
        let timeline = timelines.get_mut(&session_id).ok_or_else(|| {
            ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                format!("session not found: {session_id}"),
            )
        })?;
        let message_id =
            resolve_target_message_id(timeline, explicit_message_id).ok_or_else(|| {
                ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    "missing required parameter `messageId`",
                )
            })?;
        let now = now_ms();
        let entry = timeline
            .entries
            .iter_mut()
            .find(|entry| entry.id == message_id)
            .ok_or_else(|| {
                ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    format!("message not found: {message_id}"),
                )
            })?;
        let unpinned = if entry.pinned_at_ms.is_some() {
            entry.pinned_at_ms = None;
            true
        } else {
            false
        };
        timeline.updated_at_ms = now;
        Ok(json!({
            "status": "completed",
            "action": "unpin",
            "sessionId": session_id,
            "messageId": message_id,
            "unpinned": unpinned,
            "entry": serialize_session_entry(entry)
        }))
    }

    async fn execute_message_list_pins(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "pins")?;
        let limit = request
            .args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(50)
            .clamp(1, 200) as usize;
        let timelines = self.session_timelines.lock().await;
        let Some(timeline) = timelines.get(&session_id) else {
            return Ok(json!({
                "status": "completed",
                "action": "pins",
                "sessionId": session_id,
                "pins": [],
                "count": 0
            }));
        };
        let mut pinned_entries = timeline
            .entries
            .iter()
            .filter(|entry| entry.pinned_at_ms.is_some() && entry.deleted_at_ms.is_none())
            .collect::<Vec<_>>();
        pinned_entries.sort_by(|left, right| {
            right
                .pinned_at_ms
                .cmp(&left.pinned_at_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        if pinned_entries.len() > limit {
            pinned_entries.truncate(limit);
        }
        let pins = pinned_entries
            .into_iter()
            .map(serialize_session_entry)
            .collect::<Vec<_>>();
        Ok(json!({
            "status": "completed",
            "action": "pins",
            "sessionId": session_id,
            "pins": pins,
            "count": pins.len(),
            "limit": limit
        }))
    }

    async fn execute_message_permissions(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let mut permissions = json!({
            "send": true,
            "poll": true,
            "react": true,
            "reactions": true,
            "read": true,
            "edit": true,
            "delete": true,
            "pin": true,
            "unpin": true,
            "pins": true,
            "threadCreate": true,
            "threadList": true,
            "threadReply": true,
            "search": true,
            "memberInfo": true,
            "roleInfo": true,
            "channelInfo": true,
            "channelList": true,
            "voiceStatus": true,
            "eventList": true,
            "eventCreate": true,
            "emojiList": true,
            "emojiUpload": true,
            "stickerSend": true,
            "stickerUpload": true,
            "roleAdd": true,
            "roleRemove": true,
            "timeout": true,
            "kick": true,
            "ban": true
        });
        let mut resolved_channel: Option<String> = None;
        if let Some((channel, explicit)) = self.resolve_message_channel_for_capability(request) {
            resolved_channel = Some(channel.clone());
            let Some(capability) = self.message_channel_capabilities.get(channel.as_str()) else {
                if explicit {
                    return Err(ToolRuntimeError::new(
                        ToolRuntimeErrorCode::InvalidArgs,
                        format!("unsupported channel: {channel}"),
                    ));
                }
                return Ok(json!({
                    "status": "completed",
                    "action": "permissions",
                    "sessionId": session_id,
                    "channel": channel,
                    "permissions": permissions
                }));
            };
            permissions["poll"] =
                Value::Bool(Self::message_channel_supports_action(capability, "poll"));
            permissions["react"] =
                Value::Bool(Self::message_channel_supports_action(capability, "react"));
            permissions["reactions"] = Value::Bool(Self::message_channel_supports_action(
                capability,
                "reactions",
            ));
            permissions["read"] =
                Value::Bool(Self::message_channel_supports_action(capability, "read"));
            permissions["edit"] =
                Value::Bool(Self::message_channel_supports_action(capability, "edit"));
            permissions["delete"] =
                Value::Bool(Self::message_channel_supports_action(capability, "delete"));
            permissions["pin"] =
                Value::Bool(Self::message_channel_supports_action(capability, "pin"));
            permissions["unpin"] =
                Value::Bool(Self::message_channel_supports_action(capability, "unpin"));
            permissions["pins"] =
                Value::Bool(Self::message_channel_supports_action(capability, "pins"));
            permissions["permissions"] = Value::Bool(Self::message_channel_supports_action(
                capability,
                "permissions",
            ));
            permissions["search"] =
                Value::Bool(Self::message_channel_supports_action(capability, "search"));
            permissions["threadCreate"] = Value::Bool(Self::message_channel_supports_action(
                capability,
                "thread-create",
            ));
            permissions["threadList"] = Value::Bool(Self::message_channel_supports_action(
                capability,
                "thread-list",
            ));
            permissions["threadReply"] = Value::Bool(Self::message_channel_supports_action(
                capability,
                "thread-reply",
            ));
            permissions["memberInfo"] = Value::Bool(Self::message_channel_supports_action(
                capability,
                "member-info",
            ));
            permissions["roleInfo"] = Value::Bool(Self::message_channel_supports_action(
                capability,
                "role-info",
            ));
            permissions["channelInfo"] = Value::Bool(Self::message_channel_supports_action(
                capability,
                "channel-info",
            ));
            permissions["channelList"] = Value::Bool(Self::message_channel_supports_action(
                capability,
                "channel-list",
            ));
            permissions["voiceStatus"] = Value::Bool(Self::message_channel_supports_action(
                capability,
                "voice-status",
            ));
            permissions["eventList"] = Value::Bool(Self::message_channel_supports_action(
                capability,
                "event-list",
            ));
            permissions["eventCreate"] = Value::Bool(Self::message_channel_supports_action(
                capability,
                "event-create",
            ));
            permissions["emojiList"] = Value::Bool(Self::message_channel_supports_action(
                capability,
                "emoji-list",
            ));
            permissions["emojiUpload"] = Value::Bool(Self::message_channel_supports_action(
                capability,
                "emoji-upload",
            ));
            permissions["stickerSend"] = Value::Bool(Self::message_channel_supports_action(
                capability,
                "sticker-send",
            ));
            permissions["stickerUpload"] = Value::Bool(Self::message_channel_supports_action(
                capability,
                "sticker-upload",
            ));
            permissions["roleAdd"] = Value::Bool(Self::message_channel_supports_action(
                capability, "role-add",
            ));
            permissions["roleRemove"] = Value::Bool(Self::message_channel_supports_action(
                capability,
                "role-remove",
            ));
            permissions["timeout"] =
                Value::Bool(Self::message_channel_supports_action(capability, "timeout"));
            permissions["kick"] =
                Value::Bool(Self::message_channel_supports_action(capability, "kick"));
            permissions["ban"] =
                Value::Bool(Self::message_channel_supports_action(capability, "ban"));
        }
        Ok(json!({
            "status": "completed",
            "action": "permissions",
            "sessionId": session_id,
            "channel": resolved_channel,
            "permissions": permissions
        }))
    }

    async fn execute_message_search(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "search")?;
        let query = required_string_arg(&request.args, &["query", "q"], "query")?;
        let limit = request
            .args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(50)
            .clamp(1, 200) as usize;
        let include_deleted =
            first_bool_arg(&request.args, &["includeDeleted", "include_deleted"]).unwrap_or(false);
        let thread_id_filter =
            first_string_arg(&request.args, &["threadId", "thread_id", "thread"]);
        let guild_id = first_string_arg(&request.args, &["guildId", "guild_id"]);
        let query_normalized = query.trim().to_ascii_lowercase();

        let timelines = self.session_timelines.lock().await;
        let mut results = Vec::new();
        if let Some(timeline) = timelines.get(&session_id) {
            for entry in timeline.entries.iter().rev() {
                if !include_deleted && entry.deleted_at_ms.is_some() {
                    continue;
                }
                if thread_id_filter
                    .as_deref()
                    .is_some_and(|thread_id| entry.thread_id.as_deref() != Some(thread_id))
                {
                    continue;
                }
                if !entry
                    .message
                    .to_ascii_lowercase()
                    .contains(&query_normalized)
                {
                    continue;
                }
                results.push(serialize_session_entry(entry));
                if results.len() >= limit {
                    break;
                }
            }
        }

        Ok(json!({
            "status": "completed",
            "action": "search",
            "sessionId": session_id,
            "guildId": guild_id,
            "query": query,
            "threadId": thread_id_filter,
            "results": results,
            "count": results.len(),
            "limit": limit
        }))
    }

    async fn execute_message_thread_create(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "thread-create")?;
        let name = first_string_arg(
            &request.args,
            &["threadName", "thread_name", "name", "title"],
        )
        .ok_or_else(|| {
            ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                "missing required parameter `threadName`",
            )
        })?;
        let source_message_id = first_string_arg(&request.args, &["messageId", "message_id"]);
        let thread = ToolRuntimeMessageThread {
            id: self.next_thread_id().await,
            name,
            created_at_ms: now_ms(),
            source_message_id,
            archived: false,
        };
        let mut threads = self.session_threads.lock().await;
        let registry = threads.entry(session_id.clone()).or_default();
        registry.updated_at_ms = thread.created_at_ms;
        registry.threads.push_back(thread.clone());
        while registry.threads.len() > self.session_history_limit {
            registry.threads.pop_front();
        }
        if threads.len() > self.session_bucket_limit {
            let evict = threads
                .iter()
                .filter(|(key, _)| key.as_str() != session_id.as_str())
                .min_by_key(|(_, value)| value.updated_at_ms)
                .map(|(key, _)| key.clone());
            if let Some(evict) = evict {
                let _ = threads.remove(&evict);
            }
        }
        Ok(json!({
            "status": "completed",
            "action": "thread-create",
            "sessionId": session_id,
            "thread": serialize_message_thread(&thread)
        }))
    }

    async fn execute_message_thread_list(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "thread-list")?;
        let include_archived =
            first_bool_arg(&request.args, &["includeArchived", "include_archived"])
                .unwrap_or(false);
        let limit = request
            .args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(100)
            .clamp(1, 250) as usize;
        let threads = self.session_threads.lock().await;
        let Some(registry) = threads.get(&session_id) else {
            return Ok(json!({
                "status": "completed",
                "action": "thread-list",
                "sessionId": session_id,
                "threads": [],
                "count": 0
            }));
        };
        let mut rows = registry
            .threads
            .iter()
            .filter(|thread| include_archived || !thread.archived)
            .cloned()
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| right.created_at_ms.cmp(&left.created_at_ms));
        if rows.len() > limit {
            rows.truncate(limit);
        }
        let payload = rows
            .iter()
            .map(serialize_message_thread)
            .collect::<Vec<_>>();
        Ok(json!({
            "status": "completed",
            "action": "thread-list",
            "sessionId": session_id,
            "threads": payload,
            "count": payload.len(),
            "limit": limit
        }))
    }

    async fn execute_message_thread_reply(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "thread-reply")?;
        let explicit_thread_id =
            first_string_arg(&request.args, &["threadId", "thread_id", "thread"]);
        let thread_id = {
            let threads = self.session_threads.lock().await;
            let Some(registry) = threads.get(&session_id) else {
                return Err(ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    format!("session thread registry not found: {session_id}"),
                ));
            };
            let resolved = explicit_thread_id.or_else(|| {
                registry
                    .threads
                    .iter()
                    .rev()
                    .find(|thread| !thread.archived)
                    .map(|thread| thread.id.clone())
            });
            let Some(thread_id) = resolved else {
                return Err(ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    "missing required parameter `threadId`",
                ));
            };
            if !registry
                .threads
                .iter()
                .any(|thread| thread.id == thread_id && !thread.archived)
            {
                return Err(ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    format!("thread not found: {thread_id}"),
                ));
            }
            thread_id
        };

        let message = first_string_arg(&request.args, &["message", "content", "text", "prompt"])
            .ok_or_else(|| {
                ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    "missing required parameter `message`",
                )
            })?;
        let role =
            normalize_message_role(first_string_arg(&request.args, &["role", "author"]).as_deref());
        let (entry, count) = self
            .append_session_entry(session_id.clone(), role, message, Some(thread_id.clone()))
            .await;
        let message_id = entry
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Ok(json!({
            "status": "completed",
            "action": "thread-reply",
            "sessionId": session_id,
            "threadId": thread_id,
            "messageId": message_id,
            "entry": entry,
            "count": count
        }))
    }

    async fn execute_message_member_info(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "member-info")?;
        let guild_id = required_string_arg(&request.args, &["guildId", "guild_id"], "guildId")?;
        let user_id = required_string_arg(&request.args, &["userId", "user_id"], "userId")?;
        let key = message_member_role_key(&guild_id, &user_id);
        let roles = self
            .member_roles
            .lock()
            .await
            .get(&key)
            .cloned()
            .unwrap_or_default();
        Ok(json!({
            "status": "completed",
            "action": "member-info",
            "sessionId": session_id,
            "member": {
                "guildId": guild_id,
                "userId": user_id,
                "displayName": format!("member-{user_id}"),
                "roles": roles,
                "joinedAt": null
            }
        }))
    }

    async fn execute_message_role_info(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "role-info")?;
        let guild_id = required_string_arg(&request.args, &["guildId", "guild_id"], "guildId")?;
        let role_id = first_string_arg(&request.args, &["roleId", "role_id"]);
        let role_name = role_id
            .as_deref()
            .map(|id| format!("role-{id}"))
            .unwrap_or_else(|| "role-member".to_owned());
        Ok(json!({
            "status": "completed",
            "action": "role-info",
            "sessionId": session_id,
            "role": {
                "guildId": guild_id,
                "roleId": role_id,
                "name": role_name
            }
        }))
    }

    async fn execute_message_channel_info(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "channel-info")?;
        let target = required_string_arg(
            &request.args,
            &["target", "channelId", "channel_id"],
            "target",
        )?;
        Ok(json!({
            "status": "completed",
            "action": "channel-info",
            "sessionId": session_id,
            "channel": {
                "id": target,
                "name": "runtime-channel",
                "kind": "text",
                "archived": false
            }
        }))
    }

    async fn execute_message_channel_list(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "channel-list")?;
        let guild_id = required_string_arg(&request.args, &["guildId", "guild_id"], "guildId")?;
        Ok(json!({
            "status": "completed",
            "action": "channel-list",
            "sessionId": session_id,
            "guildId": guild_id,
            "channels": [
                {
                    "id": format!("{guild_id}-general"),
                    "name": "general",
                    "kind": "text"
                },
                {
                    "id": format!("{guild_id}-ops"),
                    "name": "ops",
                    "kind": "text"
                }
            ],
            "count": 2
        }))
    }

    async fn execute_message_voice_status(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "voice-status")?;
        let guild_id = required_string_arg(&request.args, &["guildId", "guild_id"], "guildId")?;
        let user_id = required_string_arg(&request.args, &["userId", "user_id"], "userId")?;
        Ok(json!({
            "status": "completed",
            "action": "voice-status",
            "sessionId": session_id,
            "voice": {
                "guildId": guild_id,
                "userId": user_id,
                "connected": false,
                "channelId": null,
                "muted": false,
                "deafened": false
            }
        }))
    }

    async fn execute_message_event_list(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "event-list")?;
        let guild_id = required_string_arg(&request.args, &["guildId", "guild_id"], "guildId")?;
        let limit = request
            .args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(50)
            .clamp(1, 200) as usize;
        let events = self.message_events.lock().await;
        let mut rows = events
            .get(&guild_id)
            .map(|value| value.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        rows.sort_by(|left, right| right.created_at_ms.cmp(&left.created_at_ms));
        if rows.len() > limit {
            rows.truncate(limit);
        }
        let payload = rows.iter().map(serialize_message_event).collect::<Vec<_>>();
        Ok(json!({
            "status": "completed",
            "action": "event-list",
            "sessionId": session_id,
            "guildId": guild_id,
            "events": payload,
            "count": payload.len(),
            "limit": limit
        }))
    }

    async fn execute_message_event_create(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "event-create")?;
        let guild_id = required_string_arg(&request.args, &["guildId", "guild_id"], "guildId")?;
        let event_name =
            required_string_arg(&request.args, &["eventName", "event_name"], "eventName")?;
        let start_time =
            required_string_arg(&request.args, &["startTime", "start_time"], "startTime")?;
        let event = ToolRuntimeMessageEvent {
            id: self.next_message_event_id().await,
            guild_id: guild_id.clone(),
            name: event_name,
            start_time,
            end_time: first_string_arg(&request.args, &["endTime", "end_time"]),
            description: first_string_arg(&request.args, &["desc", "description"]),
            channel_id: first_string_arg(&request.args, &["channelId", "channel_id"]),
            location: first_string_arg(&request.args, &["location"]),
            event_type: first_string_arg(&request.args, &["eventType", "event_type"])
                .unwrap_or_else(|| "external".to_owned()),
            created_at_ms: now_ms(),
        };
        let mut events = self.message_events.lock().await;
        let registry = events.entry(guild_id.clone()).or_default();
        registry.push_back(event.clone());
        while registry.len() > self.session_history_limit {
            registry.pop_front();
        }
        Ok(json!({
            "status": "completed",
            "action": "event-create",
            "sessionId": session_id,
            "event": serialize_message_event(&event)
        }))
    }

    async fn execute_message_emoji_list(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let resolved_channel =
            self.enforce_message_channel_action_support(request, "emoji-list")?;
        let guild_id = first_string_arg(&request.args, &["guildId", "guild_id"]);
        let limit = request
            .args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(100)
            .clamp(1, 250) as usize;

        let channel = resolved_channel.unwrap_or_else(|| "discord".to_owned());
        let mut rows = match channel.as_str() {
            "slack" => vec![
                json!({
                    "id": "slack-emoji-check",
                    "name": "white_check_mark",
                    "url": "https://emoji.slack-edge.com/T000/white_check_mark/1.png"
                }),
                json!({
                    "id": "slack-emoji-rocket",
                    "name": "rocket",
                    "url": "https://emoji.slack-edge.com/T000/rocket/1.png"
                }),
            ],
            _ => vec![
                json!({
                    "id": "discord-emoji-check",
                    "name": "check",
                    "animated": false
                }),
                json!({
                    "id": "discord-emoji-rocket",
                    "name": "rocket",
                    "animated": false
                }),
            ],
        };

        let registry = self.message_emojis.lock().await;
        let mut dynamic = Vec::new();
        if channel == "discord" {
            match guild_id.as_deref() {
                Some(guild_id) => {
                    let key = message_asset_registry_key("discord", Some(guild_id));
                    if let Some(emojis) = registry.get(&key) {
                        dynamic.extend(emojis.iter().map(serialize_message_emoji));
                    }
                }
                None => {
                    dynamic.extend(
                        registry
                            .iter()
                            .filter(|(key, _)| key.starts_with("discord::"))
                            .flat_map(|(_, emojis)| emojis.iter().map(serialize_message_emoji)),
                    );
                }
            }
        } else {
            let key = message_asset_registry_key("slack", None);
            if let Some(emojis) = registry.get(&key) {
                dynamic.extend(emojis.iter().map(serialize_message_emoji));
            }
        }
        rows.extend(dynamic);
        rows.sort_by(|left, right| {
            let left_name = left
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let right_name = right
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            left_name.cmp(&right_name)
        });
        if rows.len() > limit {
            rows.truncate(limit);
        }

        Ok(json!({
            "status": "completed",
            "action": "emoji-list",
            "sessionId": session_id,
            "channel": channel,
            "guildId": guild_id,
            "emojis": rows,
            "count": rows.len(),
            "limit": limit
        }))
    }

    async fn execute_message_emoji_upload(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "emoji-upload")?;
        let guild_id = required_string_arg(&request.args, &["guildId", "guild_id"], "guildId")?;
        let emoji_name = required_string_arg(
            &request.args,
            &["emojiName", "emoji_name", "name"],
            "emojiName",
        )?;
        let media = required_string_arg(&request.args, &["media", "file", "path"], "media")?;

        let emoji = ToolRuntimeMessageEmoji {
            id: self.next_emoji_id().await,
            guild_id: Some(guild_id.clone()),
            name: emoji_name,
            media: Some(media),
            created_at_ms: now_ms(),
        };
        let mut registry = self.message_emojis.lock().await;
        let key = message_asset_registry_key("discord", Some(guild_id.as_str()));
        let bucket = registry.entry(key).or_default();
        bucket.push_back(emoji.clone());
        while bucket.len() > self.session_history_limit {
            bucket.pop_front();
        }

        Ok(json!({
            "status": "completed",
            "action": "emoji-upload",
            "sessionId": session_id,
            "guildId": guild_id,
            "emoji": serialize_message_emoji(&emoji)
        }))
    }

    async fn execute_message_sticker_send(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "sticker-send")?;
        let target = required_string_arg(
            &request.args,
            &["target", "channelId", "channel_id"],
            "target",
        )?;
        let mut sticker_ids =
            first_string_list_arg(&request.args, &["stickerIds", "sticker_ids"], 8, 128);
        if sticker_ids.is_empty() {
            if let Some(single) = first_string_arg(&request.args, &["stickerId", "sticker_id"]) {
                sticker_ids.push(single);
            }
        }
        if sticker_ids.is_empty() {
            return Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                "missing required parameter `stickerId`",
            ));
        }

        let message = first_string_arg(&request.args, &["message", "content", "text"]);
        let entry_text = match message.as_deref() {
            Some(text) => format!("[sticker:{}] {text}", sticker_ids.join(",")),
            None => format!("[sticker:{}]", sticker_ids.join(",")),
        };
        let (entry, count) = self
            .append_session_entry(
                session_id.clone(),
                "assistant".to_owned(),
                entry_text,
                first_string_arg(&request.args, &["threadId", "thread_id", "thread"]),
            )
            .await;
        let message_id = entry
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();

        Ok(json!({
            "status": "completed",
            "action": "sticker-send",
            "sessionId": session_id,
            "target": target,
            "stickerIds": sticker_ids,
            "messageId": message_id,
            "entry": entry,
            "count": count
        }))
    }

    async fn execute_message_sticker_upload(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "sticker-upload")?;
        let guild_id = required_string_arg(&request.args, &["guildId", "guild_id"], "guildId")?;
        let sticker_name = required_string_arg(
            &request.args,
            &["stickerName", "sticker_name", "name"],
            "stickerName",
        )?;
        let sticker_desc = required_string_arg(
            &request.args,
            &["stickerDesc", "sticker_desc", "description", "desc"],
            "stickerDesc",
        )?;
        let sticker_tags = required_string_arg(
            &request.args,
            &["stickerTags", "sticker_tags", "tags"],
            "stickerTags",
        )?;
        let media = required_string_arg(&request.args, &["media", "file", "path"], "media")?;

        let sticker = ToolRuntimeMessageSticker {
            id: self.next_sticker_id().await,
            guild_id: guild_id.clone(),
            name: sticker_name,
            description: sticker_desc,
            tags: sticker_tags,
            media: Some(media),
            created_at_ms: now_ms(),
        };
        let mut registry = self.message_stickers.lock().await;
        let key = message_asset_registry_key("discord", Some(guild_id.as_str()));
        let bucket = registry.entry(key).or_default();
        bucket.push_back(sticker.clone());
        while bucket.len() > self.session_history_limit {
            bucket.pop_front();
        }

        Ok(json!({
            "status": "completed",
            "action": "sticker-upload",
            "sessionId": session_id,
            "guildId": guild_id,
            "sticker": serialize_message_sticker(&sticker)
        }))
    }

    async fn execute_message_role_mutation(
        &self,
        request: &ToolRuntimeRequest,
        action: &str,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, action)?;
        let guild_id = required_string_arg(&request.args, &["guildId", "guild_id"], "guildId")?;
        let user_id = required_string_arg(&request.args, &["userId", "user_id"], "userId")?;
        let role_id = required_string_arg(&request.args, &["roleId", "role_id"], "roleId")?;
        let key = message_member_role_key(&guild_id, &user_id);
        let mut members = self.member_roles.lock().await;
        let roles = members.entry(key).or_default();
        let applied = if action == "role-add" {
            if roles.iter().any(|value| value == &role_id) {
                false
            } else {
                roles.push(role_id.clone());
                roles.sort();
                true
            }
        } else {
            let before = roles.len();
            roles.retain(|value| value != &role_id);
            before != roles.len()
        };
        Ok(json!({
            "status": "completed",
            "action": action,
            "sessionId": session_id,
            "guildId": guild_id,
            "userId": user_id,
            "roleId": role_id,
            "applied": applied,
            "roles": roles
        }))
    }

    async fn execute_message_timeout(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "timeout")?;
        let guild_id = required_string_arg(&request.args, &["guildId", "guild_id"], "guildId")?;
        let user_id = required_string_arg(&request.args, &["userId", "user_id"], "userId")?;
        let duration_min = request
            .args
            .get("durationMin")
            .or_else(|| request.args.get("duration_min"))
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, 40_320));
        let until = first_string_arg(&request.args, &["until"]);
        Ok(json!({
            "status": "completed",
            "action": "timeout",
            "sessionId": session_id,
            "guildId": guild_id,
            "userId": user_id,
            "durationMin": duration_min,
            "until": until,
            "applied": duration_min.is_some() || until.is_some()
        }))
    }

    async fn execute_message_kick(&self, request: &ToolRuntimeRequest) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "kick")?;
        let guild_id = required_string_arg(&request.args, &["guildId", "guild_id"], "guildId")?;
        let user_id = required_string_arg(&request.args, &["userId", "user_id"], "userId")?;
        let reason = first_string_arg(&request.args, &["reason"]);
        Ok(json!({
            "status": "completed",
            "action": "kick",
            "sessionId": session_id,
            "guildId": guild_id,
            "userId": user_id,
            "reason": reason,
            "applied": true
        }))
    }

    async fn execute_message_ban(&self, request: &ToolRuntimeRequest) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "ban")?;
        let guild_id = required_string_arg(&request.args, &["guildId", "guild_id"], "guildId")?;
        let user_id = required_string_arg(&request.args, &["userId", "user_id"], "userId")?;
        let reason = first_string_arg(&request.args, &["reason"]);
        let delete_days = request
            .args
            .get("deleteDays")
            .or_else(|| request.args.get("delete_days"))
            .and_then(Value::as_u64)
            .map(|value| value.clamp(0, 7));
        Ok(json!({
            "status": "completed",
            "action": "ban",
            "sessionId": session_id,
            "guildId": guild_id,
            "userId": user_id,
            "reason": reason,
            "deleteDays": delete_days,
            "applied": true
        }))
    }

    async fn execute_message_react(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "react")?;
        let explicit_message_id =
            first_string_arg(&request.args, &["messageId", "message_id", "id"]);
        let remove = first_bool_arg(&request.args, &["remove", "delete"]).unwrap_or(false);
        let emoji = first_string_arg(&request.args, &["emoji", "reaction"]);
        if !remove && emoji.is_none() {
            return Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                "missing required parameter `emoji`",
            ));
        }
        let actor = first_string_arg(
            &request.args,
            &[
                "actor",
                "participant",
                "user",
                "targetAuthor",
                "target_author",
            ],
        )
        .unwrap_or_else(|| "self".to_owned());

        let mut timelines = self.session_timelines.lock().await;
        let timeline = timelines.get_mut(&session_id).ok_or_else(|| {
            ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                format!("session not found: {session_id}"),
            )
        })?;
        let message_id =
            resolve_target_message_id(timeline, explicit_message_id).ok_or_else(|| {
                ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    "missing required parameter `messageId`",
                )
            })?;
        let now = now_ms();
        let entry = timeline
            .entries
            .iter_mut()
            .find(|entry| entry.id == message_id)
            .ok_or_else(|| {
                ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    format!("message not found: {message_id}"),
                )
            })?;
        if entry.deleted_at_ms.is_some() {
            return Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                "cannot react to a deleted message",
            ));
        }

        let emoji_for_output = emoji.clone();
        let applied = if remove {
            let previous_len = entry.reactions.len();
            if let Some(emoji) = emoji.as_deref() {
                entry
                    .reactions
                    .retain(|reaction| !(reaction.actor == actor && reaction.emoji == emoji));
            } else {
                entry.reactions.retain(|reaction| reaction.actor != actor);
            }
            previous_len != entry.reactions.len()
        } else {
            let emoji = emoji.clone().unwrap_or_default();
            let duplicate = entry
                .reactions
                .iter()
                .any(|reaction| reaction.actor == actor && reaction.emoji == emoji);
            if duplicate {
                false
            } else {
                entry.reactions.push(ToolRuntimeSessionReaction {
                    emoji,
                    actor: actor.clone(),
                    created_at_ms: now,
                });
                true
            }
        };
        timeline.updated_at_ms = now;
        let reactions = serialize_session_reactions(&entry.reactions);
        Ok(json!({
            "status": "completed",
            "action": "react",
            "sessionId": session_id,
            "messageId": message_id,
            "emoji": emoji_for_output,
            "remove": remove,
            "applied": applied,
            "reactionCount": reactions.len(),
            "reactions": reactions
        }))
    }

    async fn execute_message_reactions(
        &self,
        request: &ToolRuntimeRequest,
    ) -> ToolRuntimeResult<Value> {
        let session_id = resolve_message_session_id(request);
        let _channel = self.enforce_message_channel_action_support(request, "reactions")?;
        let explicit_message_id =
            first_string_arg(&request.args, &["messageId", "message_id", "id"]);
        let timelines = self.session_timelines.lock().await;
        let timeline = timelines.get(&session_id).ok_or_else(|| {
            ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                format!("session not found: {session_id}"),
            )
        })?;
        let message_id =
            resolve_target_message_id(timeline, explicit_message_id).ok_or_else(|| {
                ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    "missing required parameter `messageId`",
                )
            })?;
        let entry = timeline
            .entries
            .iter()
            .find(|entry| entry.id == message_id)
            .ok_or_else(|| {
                ToolRuntimeError::new(
                    ToolRuntimeErrorCode::InvalidArgs,
                    format!("message not found: {message_id}"),
                )
            })?;
        let reactions = serialize_session_reactions(&entry.reactions);
        Ok(json!({
            "status": "completed",
            "action": "reactions",
            "sessionId": session_id,
            "messageId": message_id,
            "count": reactions.len(),
            "reactions": reactions
        }))
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
                "commands": TOOL_RUNTIME_NODE_COMMANDS,
                "capabilities": {
                    "actions": [
                        "status",
                        "list",
                        "describe",
                        "pending",
                        "approve",
                        "reject",
                        "notify",
                        "run",
                        "invoke"
                    ]
                }
            })),
            "list" => {
                let include_caps = request
                    .args
                    .get("includeCapabilities")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let caps = if include_caps {
                    json!(TOOL_RUNTIME_NODE_COMMANDS)
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
            "describe" => {
                let node_id =
                    required_string_arg(&request.args, &["nodeId", "node_id", "node"], "node")?;
                Ok(json!({
                    "status": "completed",
                    "node": {
                        "id": node_id,
                        "name": "Local Node",
                        "connected": true,
                        "local": true,
                        "commands": TOOL_RUNTIME_NODE_COMMANDS
                    }
                }))
            }
            "pending" => Ok(json!({
                "status": "completed",
                "pending": [],
                "count": 0
            })),
            "approve" | "reject" => {
                let request_id =
                    required_string_arg(&request.args, &["requestId", "request_id"], "requestId")?;
                let approved = action == "approve";
                Ok(json!({
                    "status": "completed",
                    "action": action,
                    "requestId": request_id,
                    "approved": approved,
                    "rejected": !approved
                }))
            }
            "notify" => {
                let node_id =
                    required_string_arg(&request.args, &["nodeId", "node_id", "node"], "node")?;
                let title = first_string_arg(&request.args, &["title", "subject", "summary"]);
                let body = first_string_arg(&request.args, &["body", "message", "text"]);
                let has_title = title
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_some();
                let has_body = body
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_some();
                if !has_title && !has_body {
                    return Err(ToolRuntimeError::new(
                        ToolRuntimeErrorCode::InvalidArgs,
                        "nodes.notify requires `title` or `body`",
                    ));
                }
                let mut params = serde_json::Map::new();
                if let Some(value) = title {
                    params.insert("title".to_owned(), Value::String(value));
                }
                if let Some(value) = body {
                    params.insert("body".to_owned(), Value::String(value));
                }
                if let Some(value) = first_string_arg(&request.args, &["sound"]) {
                    params.insert("sound".to_owned(), Value::String(value));
                }
                if let Some(value) = first_string_arg(&request.args, &["level"]) {
                    params.insert("level".to_owned(), Value::String(value));
                }
                if let Some(value) = first_string_arg(&request.args, &["priority"]) {
                    params.insert("priority".to_owned(), Value::String(value));
                }
                if let Some(value) = first_string_arg(&request.args, &["delivery"]) {
                    params.insert("delivery".to_owned(), Value::String(value));
                }
                let result = self.execute_nodes_system_notify(&Value::Object(params));
                Ok(json!({
                    "status": "completed",
                    "nodeId": node_id,
                    "command": "system.notify",
                    "result": result
                }))
            }
            "run" => {
                let node_id =
                    required_string_arg(&request.args, &["nodeId", "node_id", "node"], "node")?;
                if request.args.get("command").is_none() {
                    return Err(ToolRuntimeError::new(
                        ToolRuntimeErrorCode::InvalidArgs,
                        "nodes.run requires params.command",
                    ));
                }
                let mut params = serde_json::Map::new();
                if let Some(value) = request.args.get("command") {
                    params.insert("command".to_owned(), value.clone());
                }
                for key in [
                    "rawCommand",
                    "raw_command",
                    "cwd",
                    "workdir",
                    "timeoutMs",
                    "timeout_ms",
                    "env",
                    "needsScreenRecording",
                    "needs_screen_recording",
                ] {
                    if let Some(value) = request.args.get(key) {
                        params.insert(key.to_owned(), value.clone());
                    }
                }
                let result = self
                    .execute_nodes_system_run(request, &Value::Object(params))
                    .await?;
                Ok(json!({
                    "status": "completed",
                    "nodeId": node_id,
                    "command": "system.run",
                    "result": result
                }))
            }
            "invoke" => {
                let node_id = required_string_arg(&request.args, &["nodeId", "node_id"], "nodeId")?;
                let command = required_string_arg(&request.args, &["command"], "command")?;
                let normalized_command = command.trim().to_ascii_lowercase();
                let invoke_id = format!("tool-node-invoke-{}-{}", now_ms(), request.request_id);
                let params = node_params_from_args(&request.args);
                let result = match normalized_command.as_str() {
                    "camera.list" => json!({
                        "cameras": [
                            {
                                "id": "camera-front",
                                "name": "Front Camera",
                                "facing": "front",
                                "formats": ["jpeg", "png"]
                            },
                            {
                                "id": "camera-rear",
                                "name": "Rear Camera",
                                "facing": "rear",
                                "formats": ["jpeg", "png", "heic"]
                            }
                        ],
                        "count": 2
                    }),
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
                    "device.info" => json!({
                        "id": node_id,
                        "name": "Local Node",
                        "platform": std::env::consts::OS,
                        "os": std::env::consts::OS,
                        "arch": std::env::consts::ARCH,
                        "source": "tool-runtime"
                    }),
                    "device.status" => json!({
                        "online": true,
                        "batteryPercent": 100,
                        "charging": true,
                        "network": "wifi",
                        "ts": now_ms()
                    }),
                    "contacts.search" => {
                        let query = params
                            .get("query")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .unwrap_or("contact");
                        json!({
                            "query": query,
                            "results": [{
                                "id": "contact-1",
                                "name": format!("{query} result"),
                                "phone": "+15550001111"
                            }],
                            "count": 1
                        })
                    }
                    "calendar.events" => json!({
                        "events": [{
                            "id": "evt-1",
                            "title": "Parity Checkpoint",
                            "startTime": "2026-02-21T20:00:00Z",
                            "endTime": "2026-02-21T20:30:00Z"
                        }],
                        "count": 1
                    }),
                    "reminders.list" => json!({
                        "reminders": [{
                            "id": "rem-1",
                            "title": "Review parity artifacts",
                            "completed": false
                        }],
                        "count": 1
                    }),
                    "photos.latest" => {
                        let limit = params
                            .get("limit")
                            .and_then(Value::as_u64)
                            .unwrap_or(1)
                            .clamp(1, 10) as usize;
                        let photos = (0..limit)
                            .map(|idx| {
                                json!({
                                    "id": format!("photo-{idx}"),
                                    "mimeType": "image/jpeg",
                                    "path": format!("memory://nodes/{node_id}/photos/{idx}.jpg")
                                })
                            })
                            .collect::<Vec<_>>();
                        json!({
                            "photos": photos,
                            "count": limit
                        })
                    }
                    "motion.activity" => json!({
                        "activity": "stationary",
                        "confidence": 0.95,
                        "ts": now_ms()
                    }),
                    "motion.pedometer" => json!({
                        "steps": 0,
                        "distanceMeters": 0.0,
                        "floorsAscended": 0,
                        "floorsDescended": 0,
                        "ts": now_ms()
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
                    "canvas.hide" => json!({
                        "hidden": true,
                        "ts": now_ms()
                    }),
                    "canvas.navigate" => {
                        let url = params
                            .get("url")
                            .and_then(Value::as_str)
                            .unwrap_or("/")
                            .to_owned();
                        json!({
                            "navigated": true,
                            "url": url,
                            "ts": now_ms()
                        })
                    }
                    "canvas.eval" => {
                        let script = params
                            .get("script")
                            .or_else(|| params.get("code"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        json!({
                            "ok": true,
                            "script": script,
                            "result": Value::Null
                        })
                    }
                    "canvas.snapshot" => json!({
                        "mimeType": "image/png",
                        "bytes": 0,
                        "imageBase64": ""
                    }),
                    "canvas.a2ui.push" => {
                        let pushed = params
                            .get("items")
                            .and_then(Value::as_array)
                            .map(|items| items.len())
                            .unwrap_or(0);
                        json!({
                            "accepted": true,
                            "pushed": pushed,
                            "ts": now_ms()
                        })
                    }
                    "canvas.a2ui.pushjsonl" => {
                        let pushed = params
                            .get("jsonl")
                            .and_then(Value::as_str)
                            .map(|raw| {
                                raw.lines()
                                    .map(str::trim)
                                    .filter(|line| !line.is_empty())
                                    .count()
                            })
                            .unwrap_or(0);
                        json!({
                            "accepted": true,
                            "pushed": pushed,
                            "format": "jsonl",
                            "ts": now_ms()
                        })
                    }
                    "canvas.a2ui.reset" => json!({
                        "reset": true,
                        "ts": now_ms()
                    }),
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
        let raw_command = first_string_arg(params, &["rawCommand", "raw_command"])
            .or_else(|| first_string_arg(&request.args, &["rawCommand", "raw_command"]));
        let argv = match params.get("command") {
            Some(Value::Array(items)) => Some(parse_tool_runtime_system_run_argv(items)?),
            _ => None,
        };
        let command = if let Some(argv_values) = argv.as_ref() {
            let inferred = infer_tool_runtime_system_run_text_from_argv(argv_values);
            if let Some(raw) = raw_command.as_ref() {
                if raw.trim() != inferred {
                    return Err(ToolRuntimeError::new(
                        ToolRuntimeErrorCode::InvalidArgs,
                        "system.run rawCommand does not match command",
                    ));
                }
            }
            raw_command.clone().unwrap_or(inferred)
        } else {
            first_string_arg(params, &["command"])
                .or_else(|| first_string_arg(&request.args, &["commandText", "shell"]))
                .or(raw_command.clone())
                .ok_or_else(|| {
                    ToolRuntimeError::new(
                        ToolRuntimeErrorCode::InvalidArgs,
                        "system.run requires params.command",
                    )
                })?
        };

        if command.trim().is_empty() {
            return Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                "system.run requires params.command",
            ));
        }
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
        let outcome = run_shell_command(command.clone(), cwd).await?;
        let mut aggregated = outcome.aggregated;
        const MAX_CHARS: usize = 8_192;
        if aggregated.chars().count() > MAX_CHARS {
            aggregated = aggregated.chars().take(MAX_CHARS).collect::<String>();
        }
        Ok(json!({
            "status": outcome.status,
            "command": command,
            "rawCommand": raw_command,
            "argv": argv,
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

    async fn next_thread_id(&self) -> String {
        let mut counter = self.thread_counter.lock().await;
        *counter += 1;
        format!("thread-{:06}", *counter)
    }

    async fn next_message_event_id(&self) -> String {
        let mut counter = self.message_event_counter.lock().await;
        *counter += 1;
        format!("event-{:06}", *counter)
    }

    async fn next_emoji_id(&self) -> String {
        let mut counter = self.emoji_counter.lock().await;
        *counter += 1;
        format!("emoji-{:06}", *counter)
    }

    async fn next_sticker_id(&self) -> String {
        let mut counter = self.sticker_counter.lock().await;
        *counter += 1;
        format!("sticker-{:06}", *counter)
    }

    async fn append_session_entry(
        &self,
        session_id: String,
        role: String,
        message: String,
        thread_id: Option<String>,
    ) -> (Value, usize) {
        let entry = ToolRuntimeSessionEntry {
            id: self.next_session_entry_id().await,
            role,
            message,
            thread_id,
            created_at_ms: now_ms(),
            edited_at_ms: None,
            deleted_at_ms: None,
            pinned_at_ms: None,
            reactions: Vec::new(),
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
        (serialize_session_entry(&entry), count)
    }

    async fn session_history(&self, session_id: &str) -> (Vec<Value>, usize) {
        let timelines = self.session_timelines.lock().await;
        let Some(timeline) = timelines.get(session_id) else {
            return (Vec::new(), 0);
        };
        let entries = timeline
            .entries
            .iter()
            .map(serialize_session_entry)
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
        let removed_timeline = timelines.remove(session_id).is_some();
        drop(timelines);
        let mut threads = self.session_threads.lock().await;
        let removed_threads = threads.remove(session_id).is_some();
        removed_timeline || removed_threads
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

fn resolve_message_session_id(request: &ToolRuntimeRequest) -> String {
    first_string_arg(&request.args, &["sessionId", "session_id"])
        .unwrap_or_else(|| request.session_id.clone())
}

fn resolve_target_message_id(
    timeline: &ToolRuntimeSessionTimeline,
    explicit_message_id: Option<String>,
) -> Option<String> {
    explicit_message_id.or_else(|| {
        timeline
            .entries
            .iter()
            .rev()
            .find(|entry| entry.deleted_at_ms.is_none())
            .map(|entry| entry.id.clone())
    })
}

fn serialize_session_reactions(reactions: &[ToolRuntimeSessionReaction]) -> Vec<Value> {
    reactions
        .iter()
        .map(|reaction| {
            json!({
                "emoji": reaction.emoji,
                "actor": reaction.actor,
                "ts": reaction.created_at_ms
            })
        })
        .collect::<Vec<_>>()
}

fn serialize_message_thread(thread: &ToolRuntimeMessageThread) -> Value {
    json!({
        "id": thread.id,
        "name": thread.name,
        "ts": thread.created_at_ms,
        "sourceMessageId": thread.source_message_id,
        "archived": thread.archived
    })
}

fn serialize_message_event(event: &ToolRuntimeMessageEvent) -> Value {
    json!({
        "id": event.id,
        "guildId": event.guild_id,
        "name": event.name,
        "startTime": event.start_time,
        "endTime": event.end_time,
        "description": event.description,
        "channelId": event.channel_id,
        "location": event.location,
        "eventType": event.event_type,
        "ts": event.created_at_ms
    })
}

fn serialize_message_emoji(emoji: &ToolRuntimeMessageEmoji) -> Value {
    json!({
        "id": emoji.id,
        "guildId": emoji.guild_id,
        "name": emoji.name,
        "media": emoji.media,
        "animated": false,
        "ts": emoji.created_at_ms
    })
}

fn serialize_message_sticker(sticker: &ToolRuntimeMessageSticker) -> Value {
    json!({
        "id": sticker.id,
        "guildId": sticker.guild_id,
        "name": sticker.name,
        "description": sticker.description,
        "tags": sticker.tags,
        "media": sticker.media,
        "ts": sticker.created_at_ms
    })
}

fn message_asset_registry_key(channel: &str, guild_id: Option<&str>) -> String {
    match guild_id {
        Some(guild_id) => format!("{channel}::{guild_id}"),
        None => format!("{channel}::global"),
    }
}

fn message_member_role_key(guild_id: &str, user_id: &str) -> String {
    format!("{guild_id}::{user_id}")
}

fn serialize_session_entry(entry: &ToolRuntimeSessionEntry) -> Value {
    let reactions = serialize_session_reactions(&entry.reactions);
    json!({
        "id": entry.id,
        "role": entry.role,
        "message": entry.message,
        "threadId": entry.thread_id,
        "ts": entry.created_at_ms,
        "editedAt": entry.edited_at_ms,
        "deleted": entry.deleted_at_ms.is_some(),
        "deletedAt": entry.deleted_at_ms,
        "pinned": entry.pinned_at_ms.is_some(),
        "pinnedAt": entry.pinned_at_ms,
        "reactionCount": reactions.len(),
        "reactions": reactions
    })
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

fn first_bool_arg(root: &Value, keys: &[&str]) -> Option<bool> {
    for key in keys {
        if let Some(value) = root.get(*key).and_then(Value::as_bool) {
            return Some(value);
        }
    }
    None
}

fn first_string_list_arg(
    root: &Value,
    keys: &[&str],
    max_items: usize,
    max_len: usize,
) -> Vec<String> {
    for key in keys {
        let Some(value) = root.get(*key) else {
            continue;
        };
        if let Some(single) = value.as_str() {
            return normalize_text(Some(single.to_owned()), max_len)
                .into_iter()
                .collect::<Vec<_>>();
        }
        if let Some(items) = value.as_array() {
            return items
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|item| normalize_text(Some(item.to_owned()), max_len))
                .take(max_items)
                .collect::<Vec<_>>();
        }
    }
    Vec::new()
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

fn parse_tool_runtime_system_run_argv(items: &[Value]) -> ToolRuntimeResult<Vec<String>> {
    const MAX_ARGV_ITEMS: usize = 64;
    const MAX_ARG_LEN: usize = 512;
    let mut argv = Vec::new();
    for item in items {
        let Some(raw) = item.as_str() else {
            return Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                "system.run command array must contain only strings",
            ));
        };
        let Some(normalized) = normalize_text(Some(raw.to_owned()), MAX_ARG_LEN) else {
            return Err(ToolRuntimeError::new(
                ToolRuntimeErrorCode::InvalidArgs,
                "system.run command array entries must be non-empty strings",
            ));
        };
        argv.push(normalized);
        if argv.len() >= MAX_ARGV_ITEMS {
            break;
        }
    }
    if argv.is_empty() {
        return Err(ToolRuntimeError::new(
            ToolRuntimeErrorCode::InvalidArgs,
            "system.run command array must not be empty",
        ));
    }
    Ok(argv)
}

fn extract_tool_runtime_shell_command_from_argv(argv: &[String]) -> Option<String> {
    let token0 = argv.first()?.trim().to_ascii_lowercase();
    if matches!(
        token0.as_str(),
        "sh" | "bash" | "zsh" | "cmd" | "cmd.exe" | "powershell" | "pwsh"
    ) && matches!(
        argv.get(1).map(String::as_str),
        Some("-c") | Some("-lc") | Some("/C") | Some("/c") | Some("-Command")
    ) {
        return argv
            .get(2)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
    }
    None
}

fn quote_tool_runtime_argv_token(token: &str) -> String {
    if token.contains(' ') || token.contains('\t') || token.contains('"') || token.contains('\'') {
        format!("{token:?}")
    } else {
        token.to_owned()
    }
}

fn infer_tool_runtime_system_run_text_from_argv(argv: &[String]) -> String {
    if let Some(shell_command) = extract_tool_runtime_shell_command_from_argv(argv) {
        return shell_command;
    }
    argv.iter()
        .map(|token| quote_tool_runtime_argv_token(token))
        .collect::<Vec<_>>()
        .join(" ")
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
    async fn tool_runtime_message_tool_supports_poll_read_edit_delete_and_reactions() {
        let host = build_host(default_policy()).await;

        let send = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-send-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "send",
                    "message": "hello parity"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message send");
        let message_id = send
            .result
            .get("messageId")
            .and_then(serde_json::Value::as_str)
            .expect("message id")
            .to_owned();
        assert_eq!(
            send.result
                .get("action")
                .and_then(serde_json::Value::as_str),
            Some("send")
        );

        let react = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-react-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "react",
                    "messageId": message_id,
                    "emoji": "✅"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message react");
        assert_eq!(
            react
                .result
                .get("reactionCount")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert_eq!(
            react
                .result
                .get("applied")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

        let reactions = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-reactions-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "reactions"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message reactions");
        assert_eq!(
            reactions
                .result
                .get("count")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert_eq!(
            reactions
                .result
                .pointer("/reactions/0/emoji")
                .and_then(serde_json::Value::as_str),
            Some("✅")
        );

        let edit = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-edit-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "edit",
                    "message": "hello parity edited"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message edit");
        assert_eq!(
            edit.result
                .pointer("/entry/message")
                .and_then(serde_json::Value::as_str),
            Some("hello parity edited")
        );
        assert!(edit
            .result
            .pointer("/entry/editedAt")
            .and_then(serde_json::Value::as_u64)
            .is_some());

        let permissions = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-permissions-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "permissions"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message permissions");
        assert_eq!(
            permissions
                .result
                .pointer("/permissions/threadCreate")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

        let thread_create = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-thread-create-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "thread-create",
                    "threadName": "ops-thread",
                    "messageId": message_id
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message thread-create");
        let thread_id = thread_create
            .result
            .pointer("/thread/id")
            .and_then(serde_json::Value::as_str)
            .expect("thread id")
            .to_owned();

        let thread_reply = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-thread-reply-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "thread-reply",
                    "threadId": thread_id.clone(),
                    "message": "threaded parity message"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message thread-reply");
        assert_eq!(
            thread_reply
                .result
                .pointer("/entry/threadId")
                .and_then(serde_json::Value::as_str),
            Some(thread_id.as_str())
        );

        let thread_list = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-thread-list-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "thread-list"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message thread-list");
        assert_eq!(
            thread_list
                .result
                .pointer("/threads/0/id")
                .and_then(serde_json::Value::as_str),
            Some(thread_id.as_str())
        );

        let thread_read = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-thread-read-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "read",
                    "threadId": thread_id.clone(),
                    "limit": 10
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message thread read");
        assert_eq!(
            thread_read
                .result
                .pointer("/messages/0/message")
                .and_then(serde_json::Value::as_str),
            Some("threaded parity message")
        );

        let role_add = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-role-add-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "role-add",
                    "guildId": "guild-1",
                    "userId": "user-1",
                    "roleId": "role-ops"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message role-add");
        assert_eq!(
            role_add
                .result
                .get("applied")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

        let member_info = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-member-info-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "member-info",
                    "guildId": "guild-1",
                    "userId": "user-1"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message member-info");
        assert_eq!(
            member_info
                .result
                .pointer("/member/roles/0")
                .and_then(serde_json::Value::as_str),
            Some("role-ops")
        );

        let event_create = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-event-create-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "event-create",
                    "guildId": "guild-1",
                    "eventName": "Release Window",
                    "startTime": "2026-02-21T20:00:00Z"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message event-create");
        assert_eq!(
            event_create
                .result
                .pointer("/event/name")
                .and_then(serde_json::Value::as_str),
            Some("Release Window")
        );

        let event_list = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-event-list-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "event-list",
                    "guildId": "guild-1"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message event-list");
        assert_eq!(
            event_list
                .result
                .get("count")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );

        let timeout = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-timeout-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "timeout",
                    "guildId": "guild-1",
                    "userId": "user-1",
                    "durationMin": 30
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message timeout");
        assert_eq!(
            timeout
                .result
                .get("applied")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

        let role_remove = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-role-remove-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "role-remove",
                    "guildId": "guild-1",
                    "userId": "user-1",
                    "roleId": "role-ops"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message role-remove");
        assert_eq!(
            role_remove
                .result
                .get("applied")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

        let read = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-read-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "read",
                    "limit": 10
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message read");
        assert_eq!(
            read.result
                .pointer("/messages/0/message")
                .and_then(serde_json::Value::as_str),
            Some("hello parity edited")
        );

        let pin = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-pin-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "pin",
                    "messageId": message_id.clone()
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message pin");
        assert_eq!(
            pin.result
                .get("pinned")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

        let pins = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-pins-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "pins"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message pins");
        assert_eq!(
            pins.result.get("count").and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert_eq!(
            pins.result
                .pointer("/pins/0/id")
                .and_then(serde_json::Value::as_str),
            Some(message_id.as_str())
        );

        let unpin = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-unpin-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "unpin",
                    "messageId": message_id.clone()
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message unpin");
        assert_eq!(
            unpin
                .result
                .get("unpinned")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

        let pins_after_unpin = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-pins-2".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "list-pins"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message pins after unpin");
        assert_eq!(
            pins_after_unpin
                .result
                .get("count")
                .and_then(serde_json::Value::as_u64),
            Some(0)
        );

        let delete = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-delete-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "delete"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message delete");
        assert_eq!(
            delete
                .result
                .get("deleted")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

        let poll = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-poll-1".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "poll",
                    "question": "Lunch?",
                    "options": ["Pizza", "Sushi"],
                    "multi": true
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message poll");
        assert_eq!(
            poll.result
                .pointer("/poll/question")
                .and_then(serde_json::Value::as_str),
            Some("Lunch?")
        );
        assert_eq!(
            poll.result
                .pointer("/poll/multi")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

        let read_after_delete = host
            .execute(ToolRuntimeRequest {
                request_id: "message-parity-read-2".to_owned(),
                session_id: "message-parity".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "read",
                    "limit": 10
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message read after delete");
        assert!(read_after_delete
            .result
            .to_string()
            .contains("[poll] Lunch?"));
    }

    #[tokio::test]
    async fn tool_runtime_message_tool_enforces_channel_capabilities() {
        let host = build_host(default_policy()).await;

        let poll_ok = host
            .execute(ToolRuntimeRequest {
                request_id: "message-channel-cap-poll-ok-1".to_owned(),
                session_id: "agent:main:telegram:dm:+15550001111".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "poll",
                    "channel": "tg",
                    "question": "Deploy now?",
                    "options": ["Yes", "No"]
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("message poll on telegram");
        assert_eq!(
            poll_ok
                .result
                .pointer("/poll/question")
                .and_then(serde_json::Value::as_str),
            Some("Deploy now?")
        );

        let poll_unsupported = host
            .execute(ToolRuntimeRequest {
                request_id: "message-channel-cap-poll-unsupported-1".to_owned(),
                session_id: "agent:main:slack:dm:operator".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "poll",
                    "question": "Deploy now?",
                    "options": ["Yes", "No"]
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect_err("poll unsupported for slack");
        assert_eq!(poll_unsupported.code.as_str(), "invalid_args");
        assert!(poll_unsupported
            .message
            .contains("unsupported poll channel: slack"));

        let thread_unsupported = host
            .execute(ToolRuntimeRequest {
                request_id: "message-channel-cap-thread-unsupported-1".to_owned(),
                session_id: "agent:main:telegram:dm:+15550001111".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "thread-create",
                    "threadName": "ops-thread"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect_err("thread unsupported for telegram");
        assert_eq!(thread_unsupported.code.as_str(), "invalid_args");
        assert!(thread_unsupported
            .message
            .contains("unsupported thread-create channel: telegram"));

        let permissions_slack = host
            .execute(ToolRuntimeRequest {
                request_id: "message-channel-cap-permissions-slack-1".to_owned(),
                session_id: "agent:main:slack:dm:operator".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "permissions"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("permissions for slack");
        assert_eq!(
            permissions_slack
                .result
                .pointer("/permissions/poll")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
        assert_eq!(
            permissions_slack
                .result
                .pointer("/permissions/threadCreate")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
        assert_eq!(
            permissions_slack
                .result
                .pointer("/permissions/memberInfo")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            permissions_slack
                .result
                .pointer("/permissions/roleAdd")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
        assert_eq!(
            permissions_slack
                .result
                .pointer("/permissions/search")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
        assert_eq!(
            permissions_slack
                .result
                .pointer("/permissions/permissions")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
        assert_eq!(
            permissions_slack
                .result
                .pointer("/permissions/emojiList")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            permissions_slack
                .result
                .pointer("/permissions/emojiUpload")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );

        let permissions_telegram = host
            .execute(ToolRuntimeRequest {
                request_id: "message-channel-cap-permissions-telegram-1".to_owned(),
                session_id: "agent:main:telegram:dm:+15550001111".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "permissions"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("permissions for telegram");
        assert_eq!(
            permissions_telegram
                .result
                .pointer("/permissions/poll")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            permissions_telegram
                .result
                .pointer("/permissions/threadCreate")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
        assert_eq!(
            permissions_telegram
                .result
                .pointer("/permissions/roleAdd")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );

        let permissions_discord = host
            .execute(ToolRuntimeRequest {
                request_id: "message-channel-cap-permissions-discord-1".to_owned(),
                session_id: "agent:main:discord:group:ops".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "permissions"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("permissions for discord");
        assert_eq!(
            permissions_discord
                .result
                .pointer("/permissions/threadCreate")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            permissions_discord
                .result
                .pointer("/permissions/roleAdd")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            permissions_discord
                .result
                .pointer("/permissions/search")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            permissions_discord
                .result
                .pointer("/permissions/permissions")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            permissions_discord
                .result
                .pointer("/permissions/stickerUpload")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

        let role_add_unsupported = host
            .execute(ToolRuntimeRequest {
                request_id: "message-channel-cap-role-add-unsupported-1".to_owned(),
                session_id: "agent:main:telegram:dm:+15550001111".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "role-add",
                    "guildId": "guild-a",
                    "userId": "user-a",
                    "roleId": "role-admin"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect_err("role-add unsupported for telegram");
        assert_eq!(role_add_unsupported.code.as_str(), "invalid_args");
        assert!(role_add_unsupported
            .message
            .contains("unsupported role-add channel: telegram"));

        let _ = host
            .execute(ToolRuntimeRequest {
                request_id: "message-channel-cap-search-seed-1".to_owned(),
                session_id: "agent:main:discord:group:ops".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "send",
                    "message": "deploy checklist ready",
                    "channel": "discord"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("seed discord search message");

        let search_discord = host
            .execute(ToolRuntimeRequest {
                request_id: "message-channel-cap-search-discord-1".to_owned(),
                session_id: "agent:main:discord:group:ops".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "search",
                    "channel": "discord",
                    "query": "deploy"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("search on discord");
        assert_eq!(
            search_discord
                .result
                .get("count")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert!(search_discord
            .result
            .to_string()
            .contains("deploy checklist ready"));

        let search_unsupported = host
            .execute(ToolRuntimeRequest {
                request_id: "message-channel-cap-search-slack-unsupported-1".to_owned(),
                session_id: "agent:main:slack:dm:operator".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "search",
                    "query": "deploy"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect_err("search unsupported for slack");
        assert_eq!(search_unsupported.code.as_str(), "invalid_args");
        assert!(search_unsupported
            .message
            .contains("unsupported search channel: slack"));

        let emoji_list_slack = host
            .execute(ToolRuntimeRequest {
                request_id: "message-channel-cap-emoji-list-slack-1".to_owned(),
                session_id: "agent:main:slack:dm:operator".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "emoji-list",
                    "channel": "slack"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("emoji list on slack");
        assert_eq!(
            emoji_list_slack
                .result
                .get("count")
                .and_then(serde_json::Value::as_u64),
            Some(2)
        );

        let emoji_upload_slack = host
            .execute(ToolRuntimeRequest {
                request_id: "message-channel-cap-emoji-upload-slack-1".to_owned(),
                session_id: "agent:main:slack:dm:operator".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "emoji-upload",
                    "channel": "slack",
                    "guildId": "workspace",
                    "emojiName": "ops",
                    "media": "/tmp/ops.png"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect_err("emoji upload unsupported for slack");
        assert_eq!(emoji_upload_slack.code.as_str(), "invalid_args");
        assert!(emoji_upload_slack
            .message
            .contains("unsupported emoji-upload channel: slack"));

        let emoji_upload_discord = host
            .execute(ToolRuntimeRequest {
                request_id: "message-channel-cap-emoji-upload-discord-1".to_owned(),
                session_id: "agent:main:discord:group:ops".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "emoji-upload",
                    "channel": "discord",
                    "guildId": "guild-ops",
                    "emojiName": "opsok",
                    "media": "/tmp/opsok.png"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("emoji upload on discord");
        assert_eq!(
            emoji_upload_discord
                .result
                .pointer("/emoji/name")
                .and_then(serde_json::Value::as_str),
            Some("opsok")
        );

        let emoji_list_discord = host
            .execute(ToolRuntimeRequest {
                request_id: "message-channel-cap-emoji-list-discord-1".to_owned(),
                session_id: "agent:main:discord:group:ops".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "emoji-list",
                    "channel": "discord",
                    "guildId": "guild-ops"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("emoji list on discord");
        assert!(emoji_list_discord.result.to_string().contains("opsok"));

        let sticker_upload_discord = host
            .execute(ToolRuntimeRequest {
                request_id: "message-channel-cap-sticker-upload-discord-1".to_owned(),
                session_id: "agent:main:discord:group:ops".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "sticker-upload",
                    "channel": "discord",
                    "guildId": "guild-ops",
                    "stickerName": "ops-sticker",
                    "stickerDesc": "Ops sticker",
                    "stickerTags": "ops",
                    "media": "/tmp/ops-sticker.png"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("sticker upload on discord");
        let sticker_id = sticker_upload_discord
            .result
            .pointer("/sticker/id")
            .and_then(serde_json::Value::as_str)
            .expect("sticker id")
            .to_owned();

        let sticker_send_discord = host
            .execute(ToolRuntimeRequest {
                request_id: "message-channel-cap-sticker-send-discord-1".to_owned(),
                session_id: "agent:main:discord:group:ops".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "sticker-send",
                    "channel": "discord",
                    "target": "channel:123",
                    "stickerId": sticker_id
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("sticker send on discord");
        assert_eq!(
            sticker_send_discord
                .result
                .get("action")
                .and_then(serde_json::Value::as_str),
            Some("sticker-send")
        );

        let sticker_send_slack = host
            .execute(ToolRuntimeRequest {
                request_id: "message-channel-cap-sticker-send-slack-1".to_owned(),
                session_id: "agent:main:slack:dm:operator".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "sticker-send",
                    "channel": "slack",
                    "target": "channel:C123",
                    "stickerId": "sticker-1"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect_err("sticker send unsupported for slack");
        assert_eq!(sticker_send_slack.code.as_str(), "invalid_args");
        assert!(sticker_send_slack
            .message
            .contains("unsupported sticker-send channel: slack"));

        let unknown_channel = host
            .execute(ToolRuntimeRequest {
                request_id: "message-channel-cap-unknown-1".to_owned(),
                session_id: "message-parity-channel-cap".to_owned(),
                tool_name: "message".to_owned(),
                args: serde_json::json!({
                    "action": "poll",
                    "channel": "custom-bridge",
                    "question": "Deploy now?",
                    "options": ["Yes", "No"]
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect_err("unknown message channel rejected");
        assert_eq!(unknown_channel.code.as_str(), "invalid_args");
        assert!(unknown_channel
            .message
            .contains("unsupported channel: custom-bridge"));
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

        let nodes_list = host
            .execute(ToolRuntimeRequest {
                request_id: "nodes-list-1".to_owned(),
                session_id: "runtime-node".to_owned(),
                tool_name: "nodes".to_owned(),
                args: serde_json::json!({
                    "action": "list",
                    "includeCapabilities": true
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("nodes list");
        assert_eq!(
            nodes_list
                .result
                .pointer("/nodes/0/capabilities/0")
                .and_then(serde_json::Value::as_str),
            Some("camera.list")
        );
        assert!(nodes_list.result.to_string().contains("device.status"));

        let device_status = host
            .execute(ToolRuntimeRequest {
                request_id: "nodes-device-status-1".to_owned(),
                session_id: "runtime-node".to_owned(),
                tool_name: "nodes".to_owned(),
                args: serde_json::json!({
                    "action": "invoke",
                    "nodeId": "node-a",
                    "command": "device.status",
                    "params": {}
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("device.status invoke");
        assert_eq!(
            device_status
                .result
                .pointer("/result/online")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            device_status
                .result
                .pointer("/result/batteryPercent")
                .and_then(serde_json::Value::as_u64),
            Some(100)
        );

        let canvas_snapshot = host
            .execute(ToolRuntimeRequest {
                request_id: "nodes-canvas-snapshot-1".to_owned(),
                session_id: "runtime-node".to_owned(),
                tool_name: "nodes".to_owned(),
                args: serde_json::json!({
                    "action": "invoke",
                    "nodeId": "node-a",
                    "command": "canvas.snapshot",
                    "params": {}
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("canvas.snapshot invoke");
        assert_eq!(
            canvas_snapshot
                .result
                .pointer("/result/mimeType")
                .and_then(serde_json::Value::as_str),
            Some("image/png")
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

    #[tokio::test]
    async fn tool_runtime_nodes_tool_supports_describe_pairing_notify_and_run_actions() {
        let host = build_host(default_policy()).await;

        let describe = host
            .execute(ToolRuntimeRequest {
                request_id: "nodes-describe-1".to_owned(),
                session_id: "runtime-node".to_owned(),
                tool_name: "nodes".to_owned(),
                args: serde_json::json!({
                    "action": "describe",
                    "node": "local-node"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("nodes describe");
        assert_eq!(
            describe
                .result
                .pointer("/node/id")
                .and_then(serde_json::Value::as_str),
            Some("local-node")
        );
        assert_eq!(
            describe
                .result
                .pointer("/node/commands/0")
                .and_then(serde_json::Value::as_str),
            Some("camera.list")
        );

        let pending = host
            .execute(ToolRuntimeRequest {
                request_id: "nodes-pending-1".to_owned(),
                session_id: "runtime-node".to_owned(),
                tool_name: "nodes".to_owned(),
                args: serde_json::json!({
                    "action": "pending"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("nodes pending");
        assert_eq!(
            pending
                .result
                .get("count")
                .and_then(serde_json::Value::as_u64),
            Some(0)
        );

        let approve = host
            .execute(ToolRuntimeRequest {
                request_id: "nodes-approve-1".to_owned(),
                session_id: "runtime-node".to_owned(),
                tool_name: "nodes".to_owned(),
                args: serde_json::json!({
                    "action": "approve",
                    "requestId": "pending-1"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("nodes approve");
        assert_eq!(
            approve
                .result
                .get("approved")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

        let reject = host
            .execute(ToolRuntimeRequest {
                request_id: "nodes-reject-1".to_owned(),
                session_id: "runtime-node".to_owned(),
                tool_name: "nodes".to_owned(),
                args: serde_json::json!({
                    "action": "reject",
                    "requestId": "pending-2"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("nodes reject");
        assert_eq!(
            reject
                .result
                .get("rejected")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

        let notify = host
            .execute(ToolRuntimeRequest {
                request_id: "nodes-notify-1".to_owned(),
                session_id: "runtime-node".to_owned(),
                tool_name: "nodes".to_owned(),
                args: serde_json::json!({
                    "action": "notify",
                    "node": "local-node",
                    "title": "Parity",
                    "body": "nodes notify path"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("nodes notify");
        assert_eq!(
            notify
                .result
                .get("command")
                .and_then(serde_json::Value::as_str),
            Some("system.notify")
        );
        assert!(notify
            .result
            .pointer("/result/notificationId")
            .and_then(serde_json::Value::as_str)
            .is_some());

        let run = host
            .execute(ToolRuntimeRequest {
                request_id: "nodes-run-1".to_owned(),
                session_id: "runtime-node".to_owned(),
                tool_name: "nodes".to_owned(),
                args: if cfg!(windows) {
                    serde_json::json!({
                        "action": "run",
                        "node": "local-node",
                        "command": ["cmd", "/C", "echo nodes-run"],
                        "rawCommand": "echo nodes-run"
                    })
                } else {
                    serde_json::json!({
                        "action": "run",
                        "node": "local-node",
                        "command": ["sh", "-lc", "echo nodes-run"],
                        "rawCommand": "echo nodes-run"
                    })
                },
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect("nodes run");
        assert_eq!(
            run.result
                .pointer("/result/status")
                .and_then(serde_json::Value::as_str),
            Some("completed")
        );
        assert_eq!(
            run.result
                .pointer("/result/rawCommand")
                .and_then(serde_json::Value::as_str),
            Some("echo nodes-run")
        );
        let run_aggregated = run
            .result
            .pointer("/result/aggregated")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        assert!(run_aggregated.contains("nodes-run"));

        let mismatch = host
            .execute(ToolRuntimeRequest {
                request_id: "nodes-run-mismatch-1".to_owned(),
                session_id: "runtime-node".to_owned(),
                tool_name: "nodes".to_owned(),
                args: serde_json::json!({
                    "action": "run",
                    "node": "local-node",
                    "command": ["echo", "one"],
                    "rawCommand": "echo two"
                }),
                sandboxed: false,
                model_provider: None,
                model_id: None,
            })
            .await
            .expect_err("run should reject mismatched rawCommand");
        assert_eq!(mismatch.code.as_str(), "invalid_args");
        assert!(mismatch
            .message
            .contains("rawCommand does not match command"));
    }
}
