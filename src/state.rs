use std::collections::{hash_map::DefaultHasher, HashMap};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(feature = "sqlite-state")]
use anyhow::Context;
use anyhow::Result;
#[cfg(all(feature = "sqlite-state", test))]
use rusqlite::OptionalExtension;
#[cfg(feature = "sqlite-state")]
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use crate::types::{ActionRequest, Decision, DecisionAction};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub session_id: String,
    pub first_seen_ms: u64,
    pub last_seen_ms: u64,
    pub total_requests: u64,
    pub allowed_count: u64,
    pub review_count: u64,
    pub blocked_count: u64,
    pub last_action: DecisionAction,
    pub last_risk_score: u8,
    pub last_source: String,
    pub last_channel: Option<String>,
}

pub struct SessionStateStore {
    backend: SessionStateBackend,
}

enum SessionStateBackend {
    Json {
        path: PathBuf,
        sessions: RwLock<HashMap<String, SessionState>>,
    },
    #[cfg(feature = "sqlite-state")]
    Sqlite { path: PathBuf, lock: Mutex<()> },
}

impl SessionStateStore {
    pub async fn new(path: PathBuf) -> Result<Self> {
        if is_sqlite_path(&path) {
            #[cfg(feature = "sqlite-state")]
            {
                init_sqlite(path.clone()).await?;
                return Ok(Self {
                    backend: SessionStateBackend::Sqlite {
                        path,
                        lock: Mutex::new(()),
                    },
                });
            }

            #[cfg(not(feature = "sqlite-state"))]
            {
                anyhow::bail!(
                    "sqlite state backend requested for {} but binary was built without \
                     `sqlite-state` feature",
                    path.display()
                );
            }
        }

        let sessions = if path.exists() {
            let text = tokio::fs::read_to_string(&path).await.unwrap_or_default();
            serde_json::from_str::<HashMap<String, SessionState>>(&text).unwrap_or_default()
        } else {
            HashMap::new()
        };

        Ok(Self {
            backend: SessionStateBackend::Json {
                path,
                sessions: RwLock::new(sessions),
            },
        })
    }

    pub async fn record(&self, request: &ActionRequest, decision: &Decision) -> Result<()> {
        let session_id = request
            .session_id
            .clone()
            .unwrap_or_else(|| "global".to_owned());
        let now = now_ms();
        let (allow_delta, review_delta, block_delta) = action_deltas(decision.action);

        match &self.backend {
            SessionStateBackend::Json { path, sessions } => {
                {
                    let mut write_guard = sessions.write().await;
                    let entry =
                        write_guard
                            .entry(session_id.clone())
                            .or_insert_with(|| SessionState {
                                session_id: session_id.clone(),
                                first_seen_ms: now,
                                last_seen_ms: now,
                                total_requests: 0,
                                allowed_count: 0,
                                review_count: 0,
                                blocked_count: 0,
                                last_action: DecisionAction::Allow,
                                last_risk_score: 0,
                                last_source: "unknown".to_owned(),
                                last_channel: None,
                            });

                    entry.last_seen_ms = now;
                    entry.total_requests += 1;
                    entry.allowed_count += allow_delta as u64;
                    entry.review_count += review_delta as u64;
                    entry.blocked_count += block_delta as u64;
                    entry.last_action = decision.action;
                    entry.last_risk_score = decision.risk_score;
                    entry.last_source = request.source.clone();
                    entry.last_channel = request.channel.clone();
                }
                persist_json(path, sessions).await?;
            }
            #[cfg(feature = "sqlite-state")]
            SessionStateBackend::Sqlite { path, lock } => {
                let _guard = lock.lock().await;
                let path = path.clone();
                let source = request.source.clone();
                let channel = request.channel.clone();
                let action = action_to_str(decision.action).to_owned();
                let risk = i64::from(decision.risk_score);
                tokio::task::spawn_blocking(move || -> Result<()> {
                    let conn = Connection::open(&path)
                        .with_context(|| format!("open sqlite {}", path.display()))?;
                    conn.pragma_update(None, "journal_mode", "WAL")
                        .with_context(|| "set WAL mode")?;
                    conn.execute_batch(SESSION_STATE_SCHEMA)
                        .with_context(|| "ensure session_state schema")?;
                    conn.execute(
                        "INSERT INTO session_state (
                            session_id,
                            first_seen_ms,
                            last_seen_ms,
                            total_requests,
                            allowed_count,
                            review_count,
                            blocked_count,
                            last_action,
                            last_risk_score,
                            last_source,
                            last_channel
                        ) VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                        ON CONFLICT(session_id) DO UPDATE SET
                            last_seen_ms=excluded.last_seen_ms,
                            total_requests=session_state.total_requests + 1,
                            allowed_count=session_state.allowed_count + excluded.allowed_count,
                            review_count=session_state.review_count + excluded.review_count,
                            blocked_count=session_state.blocked_count + excluded.blocked_count,
                            last_action=excluded.last_action,
                            last_risk_score=excluded.last_risk_score,
                            last_source=excluded.last_source,
                            last_channel=excluded.last_channel",
                        params![
                            session_id,
                            now as i64,
                            now as i64,
                            i64::from(allow_delta),
                            i64::from(review_delta),
                            i64::from(block_delta),
                            action,
                            risk,
                            source,
                            channel
                        ],
                    )
                    .with_context(|| "upsert session_state row")?;
                    Ok(())
                })
                .await
                .with_context(|| "sqlite record join error")??;
            }
        }
        Ok(())
    }

    #[cfg(test)]
    async fn get(&self, session_id: &str) -> Option<SessionState> {
        match &self.backend {
            SessionStateBackend::Json { sessions, .. } => {
                let read_guard = sessions.read().await;
                read_guard.get(session_id).cloned()
            }
            #[cfg(feature = "sqlite-state")]
            SessionStateBackend::Sqlite { path, lock } => {
                let _guard = lock.lock().await;
                let path = path.clone();
                let key = session_id.to_owned();
                tokio::task::spawn_blocking(move || -> Option<SessionState> {
                    let conn = Connection::open(path).ok()?;
                    conn.query_row(
                        "SELECT
                            session_id,
                            first_seen_ms,
                            last_seen_ms,
                            total_requests,
                            allowed_count,
                            review_count,
                            blocked_count,
                            last_action,
                            last_risk_score,
                            last_source,
                            last_channel
                         FROM session_state
                         WHERE session_id = ?1",
                        params![key],
                        |row| {
                            let action_str: String = row.get(7)?;
                            Ok(SessionState {
                                session_id: row.get(0)?,
                                first_seen_ms: row.get::<_, i64>(1)? as u64,
                                last_seen_ms: row.get::<_, i64>(2)? as u64,
                                total_requests: row.get::<_, i64>(3)? as u64,
                                allowed_count: row.get::<_, i64>(4)? as u64,
                                review_count: row.get::<_, i64>(5)? as u64,
                                blocked_count: row.get::<_, i64>(6)? as u64,
                                last_action: str_to_action(&action_str),
                                last_risk_score: row.get::<_, i64>(8)? as u8,
                                last_source: row.get(9)?,
                                last_channel: row.get(10)?,
                            })
                        },
                    )
                    .optional()
                    .ok()
                    .flatten()
                })
                .await
                .ok()
                .flatten()
            }
        }
    }
}

#[cfg(feature = "sqlite-state")]
const SESSION_STATE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS session_state (
    session_id TEXT PRIMARY KEY,
    first_seen_ms INTEGER NOT NULL,
    last_seen_ms INTEGER NOT NULL,
    total_requests INTEGER NOT NULL,
    allowed_count INTEGER NOT NULL,
    review_count INTEGER NOT NULL,
    blocked_count INTEGER NOT NULL,
    last_action TEXT NOT NULL,
    last_risk_score INTEGER NOT NULL,
    last_source TEXT NOT NULL,
    last_channel TEXT
);
"#;

#[cfg(feature = "sqlite-state")]
async fn init_sqlite(path: PathBuf) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn =
            Connection::open(&path).with_context(|| format!("open sqlite {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .with_context(|| "set WAL mode")?;
        conn.execute_batch(SESSION_STATE_SCHEMA)
            .with_context(|| "ensure session_state schema")?;
        Ok(())
    })
    .await
    .with_context(|| "sqlite init join error")??;
    Ok(())
}

async fn persist_json(
    path: &PathBuf,
    sessions: &RwLock<HashMap<String, SessionState>>,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let snapshot = {
        let read_guard = sessions.read().await;
        serde_json::to_vec_pretty(&*read_guard)?
    };
    tokio::fs::write(path, snapshot).await?;
    Ok(())
}

fn is_sqlite_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()).map(|s| s.to_ascii_lowercase()),
        Some(ext) if ext == "db" || ext == "sqlite" || ext == "sqlite3"
    )
}

fn action_deltas(action: DecisionAction) -> (u8, u8, u8) {
    match action {
        DecisionAction::Allow => (1, 0, 0),
        DecisionAction::Review => (0, 1, 0),
        DecisionAction::Block => (0, 0, 1),
    }
}

#[cfg(feature = "sqlite-state")]
fn action_to_str(action: DecisionAction) -> &'static str {
    match action {
        DecisionAction::Allow => "allow",
        DecisionAction::Review => "review",
        DecisionAction::Block => "block",
    }
}

#[cfg(all(feature = "sqlite-state", test))]
fn str_to_action(s: &str) -> DecisionAction {
    match s {
        "allow" => DecisionAction::Allow,
        "review" => DecisionAction::Review,
        "block" => DecisionAction::Block,
        _ => DecisionAction::Review,
    }
}

#[derive(Clone)]
struct IdempotencyEntry {
    decision: Decision,
    expires_at_ms: u64,
}

pub struct IdempotencyCache {
    ttl: Duration,
    max_entries: usize,
    entries: Mutex<HashMap<String, IdempotencyEntry>>,
}

impl IdempotencyCache {
    pub fn new(ttl: Duration, max_entries: usize) -> Self {
        Self {
            ttl,
            max_entries: max_entries.max(32),
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn key_for_request(request: &ActionRequest) -> String {
        if request.id != "unknown" && !request.id.trim().is_empty() {
            format!("id:{}", request.id)
        } else {
            let mut h = DefaultHasher::new();
            request.session_id.hash(&mut h);
            request.prompt.hash(&mut h);
            request.command.hash(&mut h);
            request.tool_name.hash(&mut h);
            request.channel.hash(&mut h);
            request.url.hash(&mut h);
            request.file_path.hash(&mut h);
            format!("sig:{:x}", h.finish())
        }
    }

    pub async fn get(&self, key: &str) -> Option<Decision> {
        let now = now_ms();
        let mut entries = self.entries.lock().await;
        entries.retain(|_, value| value.expires_at_ms > now);
        entries.get(key).map(|entry| entry.decision.clone())
    }

    pub async fn put(&self, key: String, decision: Decision) {
        let now = now_ms();
        let mut entries = self.entries.lock().await;
        entries.retain(|_, value| value.expires_at_ms > now);

        if entries.len() >= self.max_entries {
            if let Some(oldest_key) = entries
                .iter()
                .min_by_key(|(_, v)| v.expires_at_ms)
                .map(|(k, _)| k.clone())
            {
                entries.remove(&oldest_key);
            }
        }

        entries.insert(
            key,
            IdempotencyEntry {
                decision,
                expires_at_ms: now + self.ttl.as_millis() as u64,
            },
        );
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "sqlite-state")]
    use std::path::Path;
    use std::time::Duration;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{IdempotencyCache, SessionStateStore};
    use crate::types::{ActionRequest, Decision, DecisionAction};

    fn temp_state_path(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        p.push(format!("openclaw-rs-state-{name}-{stamp}.json"));
        p
    }

    #[cfg(feature = "sqlite-state")]
    fn temp_sqlite_state_path(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        p.push(format!("openclaw-rs-state-{name}-{stamp}.db"));
        p
    }

    fn sample_request() -> ActionRequest {
        ActionRequest {
            id: "req-1".to_owned(),
            source: "agent".to_owned(),
            session_id: Some("s-1".to_owned()),
            prompt: Some("hello".to_owned()),
            command: None,
            tool_name: Some("browser".to_owned()),
            channel: Some("discord".to_owned()),
            url: None,
            file_path: None,
            raw: serde_json::json!({}),
        }
    }

    fn sample_decision() -> Decision {
        Decision {
            action: DecisionAction::Review,
            risk_score: 44,
            reasons: vec!["test".to_owned()],
            tags: vec!["x".to_owned()],
            source: "openclaw-agent-rs".to_owned(),
        }
    }

    #[cfg(feature = "sqlite-state")]
    fn sample_request_for(
        request_id: &str,
        session_id: &str,
        source: &str,
        channel: Option<&str>,
    ) -> ActionRequest {
        ActionRequest {
            id: request_id.to_owned(),
            source: source.to_owned(),
            session_id: Some(session_id.to_owned()),
            prompt: Some("hello".to_owned()),
            command: None,
            tool_name: Some("browser".to_owned()),
            channel: channel.map(ToOwned::to_owned),
            url: None,
            file_path: None,
            raw: serde_json::json!({}),
        }
    }

    #[cfg(feature = "sqlite-state")]
    fn decision_for(action: DecisionAction, risk_score: u8) -> Decision {
        Decision {
            action,
            risk_score,
            reasons: vec!["fixture".to_owned()],
            tags: vec!["cp2".to_owned()],
            source: "openclaw-agent-rs".to_owned(),
        }
    }

    #[cfg(feature = "sqlite-state")]
    async fn cleanup_sqlite_artifacts(path: &Path) {
        let _ = tokio::fs::remove_file(path).await;
        let wal = std::path::PathBuf::from(format!("{}-wal", path.display()));
        let shm = std::path::PathBuf::from(format!("{}-shm", path.display()));
        let _ = tokio::fs::remove_file(wal).await;
        let _ = tokio::fs::remove_file(shm).await;
    }

    #[tokio::test]
    async fn records_session_counters_json() {
        let path = temp_state_path("record-json");
        let store = SessionStateStore::new(path.clone()).await.expect("store");
        store
            .record(&sample_request(), &sample_decision())
            .await
            .expect("record");
        let state = store.get("s-1").await.expect("state");
        assert_eq!(state.total_requests, 1);
        assert_eq!(state.review_count, 1);
        assert_eq!(state.last_action, DecisionAction::Review);
        assert_eq!(state.last_channel.as_deref(), Some("discord"));
        let _ = tokio::fs::remove_file(path).await;
    }

    #[cfg(feature = "sqlite-state")]
    #[tokio::test]
    async fn records_session_counters_sqlite() {
        let path = temp_sqlite_state_path("record-sqlite");
        let store = SessionStateStore::new(path.clone()).await.expect("store");
        store
            .record(&sample_request(), &sample_decision())
            .await
            .expect("record");
        let state = store.get("s-1").await.expect("state");
        assert_eq!(state.total_requests, 1);
        assert_eq!(state.review_count, 1);
        assert_eq!(state.last_action, DecisionAction::Review);
        assert_eq!(state.last_channel.as_deref(), Some("discord"));
        cleanup_sqlite_artifacts(&path).await;
    }

    #[cfg(feature = "sqlite-state")]
    #[tokio::test]
    async fn sqlite_state_survives_restart_and_continues_counters() {
        let path = temp_sqlite_state_path("restart-sqlite");
        let session_id = "s-restart";

        {
            let store = SessionStateStore::new(path.clone()).await.expect("store");
            store
                .record(
                    &sample_request_for("req-r1", session_id, "agent", Some("discord")),
                    &decision_for(DecisionAction::Review, 44),
                )
                .await
                .expect("record review");
            store
                .record(
                    &sample_request_for("req-r2", session_id, "worker", Some("slack")),
                    &decision_for(DecisionAction::Allow, 8),
                )
                .await
                .expect("record allow");

            let first = store.get(session_id).await.expect("first state");
            assert_eq!(first.total_requests, 2);
            assert_eq!(first.review_count, 1);
            assert_eq!(first.allowed_count, 1);
            assert_eq!(first.blocked_count, 0);
            assert_eq!(first.last_action, DecisionAction::Allow);
            assert_eq!(first.last_source, "worker");
            assert_eq!(first.last_channel.as_deref(), Some("slack"));
        }

        let reopened = SessionStateStore::new(path.clone())
            .await
            .expect("reopen store");
        let after_restart = reopened.get(session_id).await.expect("state after restart");
        assert_eq!(after_restart.total_requests, 2);
        assert_eq!(after_restart.review_count, 1);
        assert_eq!(after_restart.allowed_count, 1);
        assert_eq!(after_restart.blocked_count, 0);
        assert_eq!(after_restart.last_action, DecisionAction::Allow);
        assert_eq!(after_restart.last_source, "worker");
        assert_eq!(after_restart.last_channel.as_deref(), Some("slack"));

        reopened
            .record(
                &sample_request_for("req-r3", session_id, "agent", Some("discord")),
                &decision_for(DecisionAction::Block, 92),
            )
            .await
            .expect("record block");
        let final_state = reopened.get(session_id).await.expect("final state");
        assert_eq!(final_state.total_requests, 3);
        assert_eq!(final_state.review_count, 1);
        assert_eq!(final_state.allowed_count, 1);
        assert_eq!(final_state.blocked_count, 1);
        assert_eq!(final_state.last_action, DecisionAction::Block);
        assert_eq!(final_state.last_source, "agent");
        assert_eq!(final_state.last_channel.as_deref(), Some("discord"));

        cleanup_sqlite_artifacts(&path).await;
    }

    #[cfg(feature = "sqlite-state")]
    #[tokio::test]
    async fn sqlite_state_recovers_multiple_sessions_after_restart() {
        let path = temp_sqlite_state_path("restart-multi");

        {
            let store = SessionStateStore::new(path.clone()).await.expect("store");
            store
                .record(
                    &sample_request_for("req-a1", "s-a", "agent", Some("discord")),
                    &decision_for(DecisionAction::Allow, 5),
                )
                .await
                .expect("record a1");
            store
                .record(
                    &sample_request_for("req-b1", "s-b", "worker", Some("telegram")),
                    &decision_for(DecisionAction::Review, 45),
                )
                .await
                .expect("record b1");
        }

        let reopened = SessionStateStore::new(path.clone())
            .await
            .expect("reopen store");
        let a = reopened.get("s-a").await.expect("state a");
        let b = reopened.get("s-b").await.expect("state b");
        assert_eq!(a.total_requests, 1);
        assert_eq!(a.allowed_count, 1);
        assert_eq!(a.review_count, 0);
        assert_eq!(a.last_channel.as_deref(), Some("discord"));
        assert_eq!(b.total_requests, 1);
        assert_eq!(b.allowed_count, 0);
        assert_eq!(b.review_count, 1);
        assert_eq!(b.last_channel.as_deref(), Some("telegram"));

        cleanup_sqlite_artifacts(&path).await;
    }

    #[tokio::test]
    async fn reuses_cached_decision_by_request_id() {
        let cache = IdempotencyCache::new(Duration::from_secs(60), 128);
        let req = ActionRequest {
            id: "req-abc".to_owned(),
            source: "agent".to_owned(),
            session_id: Some("s-1".to_owned()),
            prompt: None,
            command: Some("git status".to_owned()),
            tool_name: Some("exec".to_owned()),
            channel: None,
            url: None,
            file_path: None,
            raw: serde_json::json!({}),
        };
        let key = IdempotencyCache::key_for_request(&req);
        let decision = Decision {
            action: DecisionAction::Allow,
            risk_score: 20,
            reasons: vec![],
            tags: vec![],
            source: "openclaw-agent-rs".to_owned(),
        };

        cache.put(key.clone(), decision.clone()).await;
        let cached = cache.get(&key).await.expect("cached");
        assert_eq!(cached.action, decision.action);
        assert_eq!(cached.risk_score, decision.risk_score);
    }
}
