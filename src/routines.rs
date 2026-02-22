use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::config::ToolRuntimeRoutinePolicyConfig;
use crate::types::ActionRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineTriggerKind {
    Manual,
    Cron,
    Event,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineDefinition {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger: RoutineTriggerKind,
    pub schedule: Option<String>,
    pub event: Option<String>,
    pub session_id: Option<String>,
    pub prompt: Option<String>,
    pub command: Option<String>,
    pub tool_name: Option<String>,
    #[serde(default)]
    pub args: Value,
    pub cooldown_secs: u64,
    pub max_parallel: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineRunRecord {
    pub run_id: String,
    pub routine_id: String,
    pub status: String,
    pub queued_at_ms: u64,
    pub source: String,
    pub request_id: String,
}

pub struct RoutineOrchestrator {
    routines: Mutex<HashMap<String, RoutineDefinition>>,
    history: Mutex<VecDeque<RoutineRunRecord>>,
    history_limit: usize,
    max_parallel_default: usize,
}

pub struct RoutineRunOutcome {
    pub request: ActionRequest,
    pub record: RoutineRunRecord,
}

impl RoutineOrchestrator {
    pub fn new(cfg: &ToolRuntimeRoutinePolicyConfig) -> Self {
        Self {
            routines: Mutex::new(HashMap::new()),
            history: Mutex::new(VecDeque::new()),
            history_limit: cfg.history_limit.max(1),
            max_parallel_default: cfg.max_parallel.max(1),
        }
    }

    pub async fn upsert(&self, mut definition: RoutineDefinition) -> RoutineDefinition {
        if definition.max_parallel == 0 {
            definition.max_parallel = self.max_parallel_default;
        }

        let mut routines = self.routines.lock().await;
        routines.insert(definition.id.clone(), definition.clone());
        definition
    }

    pub async fn remove(&self, routine_id: &str) -> Option<RoutineDefinition> {
        let mut routines = self.routines.lock().await;
        routines.remove(routine_id)
    }

    pub async fn list(&self) -> Vec<RoutineDefinition> {
        let mut rows = self
            .routines
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        rows
    }

    pub async fn history(&self) -> Vec<RoutineRunRecord> {
        self.history.lock().await.iter().cloned().collect()
    }

    pub async fn run_now(
        &self,
        routine_id: &str,
        request_id: &str,
        source: &str,
    ) -> anyhow::Result<RoutineRunOutcome> {
        let definition = {
            let routines = self.routines.lock().await;
            routines
                .get(routine_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("routine `{routine_id}` not found"))?
        };

        if !definition.enabled {
            anyhow::bail!("routine `{routine_id}` is disabled");
        }

        let now = now_ms();
        let action_request = ActionRequest {
            id: format!("routine-run-{routine_id}-{now}"),
            source: source.to_owned(),
            session_id: definition.session_id.clone(),
            prompt: definition.prompt.clone(),
            command: definition.command.clone(),
            tool_name: definition.tool_name.clone(),
            channel: None,
            url: None,
            file_path: None,
            raw: serde_json::json!({
                "routineId": routine_id,
                "trigger": "manual",
                "args": definition.args,
            }),
        };

        let record = RoutineRunRecord {
            run_id: format!("run-{routine_id}-{now}"),
            routine_id: routine_id.to_owned(),
            status: "queued".to_owned(),
            queued_at_ms: now,
            source: source.to_owned(),
            request_id: request_id.to_owned(),
        };

        self.push_history(record.clone()).await;

        Ok(RoutineRunOutcome {
            request: action_request,
            record,
        })
    }

    async fn push_history(&self, row: RoutineRunRecord) {
        let mut history = self.history.lock().await;
        history.push_back(row);
        while history.len() > self.history_limit {
            history.pop_front();
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
