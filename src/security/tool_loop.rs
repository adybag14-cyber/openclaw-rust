use std::collections::{hash_map::DefaultHasher, HashMap, VecDeque};
use std::hash::{Hash, Hasher};

use tokio::sync::Mutex;

use crate::config::ToolLoopDetectionConfig;
use crate::types::ActionRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolLoopLevel {
    Warning,
    Critical,
}

#[derive(Debug, Clone)]
pub struct ToolLoopAlert {
    pub level: ToolLoopLevel,
    pub count: usize,
}

#[derive(Debug, Clone)]
struct ToolLoopEntry {
    tool_name: String,
    fingerprint: u64,
}

pub struct ToolLoopGuard {
    cfg: ToolLoopDetectionConfig,
    by_session: Mutex<HashMap<String, VecDeque<ToolLoopEntry>>>,
}

impl ToolLoopGuard {
    pub fn new(cfg: ToolLoopDetectionConfig) -> Self {
        Self {
            cfg,
            by_session: Mutex::new(HashMap::new()),
        }
    }

    pub async fn observe(&self, request: &ActionRequest) -> Option<ToolLoopAlert> {
        if !self.cfg.enabled {
            return None;
        }

        let tool_name = normalize_tool_name(request.tool_name.as_deref()?);
        if tool_name.is_empty() {
            return None;
        }

        let session_id = request
            .session_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("global")
            .to_owned();

        let fingerprint = request_fingerprint(request, &tool_name);
        let mut guard = self.by_session.lock().await;
        let queue = guard.entry(session_id).or_default();
        queue.push_back(ToolLoopEntry {
            tool_name: tool_name.clone(),
            fingerprint,
        });
        while queue.len() > self.cfg.history_size {
            queue.pop_front();
        }

        let mut streak = 0usize;
        for entry in queue.iter().rev() {
            if entry.tool_name == tool_name && entry.fingerprint == fingerprint {
                streak += 1;
            } else {
                break;
            }
        }

        if streak >= self.cfg.critical_threshold {
            return Some(ToolLoopAlert {
                level: ToolLoopLevel::Critical,
                count: streak,
            });
        }
        if streak >= self.cfg.warning_threshold {
            return Some(ToolLoopAlert {
                level: ToolLoopLevel::Warning,
                count: streak,
            });
        }

        None
    }
}

fn request_fingerprint(request: &ActionRequest, tool_name: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    tool_name.hash(&mut hasher);
    request.command.hash(&mut hasher);
    request.prompt.hash(&mut hasher);
    request.url.hash(&mut hasher);
    request.file_path.hash(&mut hasher);
    hasher.finish()
}

fn normalize_tool_name(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "bash" => "exec".to_owned(),
        "apply-patch" => "apply_patch".to_owned(),
        _ => normalized,
    }
}

#[cfg(test)]
mod tests {
    use crate::config::ToolLoopDetectionConfig;
    use crate::types::ActionRequest;

    use super::{ToolLoopGuard, ToolLoopLevel};

    fn request_for(id: &str, command: &str) -> ActionRequest {
        ActionRequest {
            id: id.to_owned(),
            source: "test".to_owned(),
            session_id: Some("s-loop".to_owned()),
            prompt: Some("run command".to_owned()),
            command: Some(command.to_owned()),
            tool_name: Some("exec".to_owned()),
            channel: None,
            url: None,
            file_path: None,
            raw: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn emits_warning_and_critical_on_repeated_identical_tool_calls() {
        let guard = ToolLoopGuard::new(ToolLoopDetectionConfig {
            enabled: true,
            history_size: 10,
            warning_threshold: 2,
            critical_threshold: 3,
        });

        let first = guard.observe(&request_for("a-1", "git status")).await;
        assert!(first.is_none());

        let second = guard.observe(&request_for("a-2", "git status")).await;
        let second = second.expect("warning");
        assert_eq!(second.level, ToolLoopLevel::Warning);
        assert_eq!(second.count, 2);

        let third = guard.observe(&request_for("a-3", "git status")).await;
        let third = third.expect("critical");
        assert_eq!(third.level, ToolLoopLevel::Critical);
        assert_eq!(third.count, 3);
    }

    #[tokio::test]
    async fn resets_streak_when_tool_arguments_change() {
        let guard = ToolLoopGuard::new(ToolLoopDetectionConfig {
            enabled: true,
            history_size: 10,
            warning_threshold: 2,
            critical_threshold: 3,
        });

        guard.observe(&request_for("b-1", "git status")).await;
        guard.observe(&request_for("b-2", "git status")).await;
        let reset = guard.observe(&request_for("b-3", "git diff")).await;
        assert!(reset.is_none());
    }

    #[tokio::test]
    async fn disabled_loop_guard_returns_no_alert() {
        let guard = ToolLoopGuard::new(ToolLoopDetectionConfig {
            enabled: false,
            history_size: 10,
            warning_threshold: 2,
            critical_threshold: 3,
        });

        let alert = guard.observe(&request_for("c-1", "git status")).await;
        assert!(alert.is_none());
    }
}
