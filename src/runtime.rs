use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::signal;
use tracing::info;

use crate::bridge::GatewayBridge;
use crate::config::{Config, GatewayRuntimeMode};
use crate::gateway_server::GatewayServer;
use crate::memory;
use crate::security::{ActionEvaluator, DefenderEngine};
use crate::telegram_bridge;

pub struct AgentRuntime {
    config: Config,
    config_path: Option<PathBuf>,
    evaluator: Arc<dyn ActionEvaluator>,
}

impl AgentRuntime {
    pub async fn new(config: Config, config_path: Option<PathBuf>) -> Result<Self> {
        let evaluator: Arc<dyn ActionEvaluator> = DefenderEngine::new(config.clone()).await?;
        Ok(Self {
            config,
            config_path,
            evaluator,
        })
    }

    pub async fn run(self) -> Result<()> {
        tokio::spawn(memory::run_sampler(self.config.runtime.memory_sample_secs));

        info!(
            "starting runtime (mode={:?}, audit_only={}, workers={}, max_queue={}, queue_mode={:?}, group_activation={:?}, idem_ttl_s={}, idem_max={})",
            self.config.gateway.runtime_mode,
            self.config.runtime.audit_only,
            self.config.runtime.worker_concurrency,
            self.config.runtime.max_queue,
            self.config.runtime.session_queue_mode,
            self.config.runtime.group_activation_mode,
            self.config.runtime.idempotency_ttl_secs,
            self.config.runtime.idempotency_max_entries
        );

        match self.config.gateway.runtime_mode {
            GatewayRuntimeMode::BridgeClient => {
                let bridge = GatewayBridge::new(
                    self.config.gateway.clone(),
                    self.config.runtime.decision_event.clone(),
                    self.config.runtime.max_queue,
                    self.config.runtime.session_queue_mode,
                    self.config.runtime.group_activation_mode,
                );
                tokio::select! {
                    res = bridge.run_forever(self.evaluator.clone()) => res,
                    _ = signal::ctrl_c() => {
                        info!("received ctrl-c, shutting down");
                        Ok(())
                    }
                }
            }
            GatewayRuntimeMode::StandaloneServer => {
                let telegram_bridge_task = telegram_bridge::spawn(
                    self.config.gateway.clone(),
                    self.config.runtime.session_state_path.clone(),
                );
                let server = GatewayServer::new(
                    self.config.gateway.clone(),
                    self.config.runtime.decision_event.clone(),
                    self.config.runtime.max_queue,
                    self.config.runtime.session_queue_mode,
                    self.config.runtime.group_activation_mode,
                );
                let result = tokio::select! {
                    res = server.run_forever(self.evaluator.clone(), self.config_path.clone()) => res,
                    _ = signal::ctrl_c() => {
                        info!("received ctrl-c, shutting down");
                        Ok(())
                    }
                };
                telegram_bridge_task.abort();
                let _ = telegram_bridge_task.await;
                result
            }
        }
    }
}
