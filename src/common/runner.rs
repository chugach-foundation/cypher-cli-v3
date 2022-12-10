use log::{info, warn};
use std::{error, sync::Arc};
use thiserror::Error;
use tokio::{
    sync::broadcast::Sender,
    time::{self, *},
};

use super::{
    context::manager::ContextManager,
    strategy::{Strategy, StrategyError},
};

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("Strategy error: {:?}", self)]
    StrategyError(StrategyError),
}

pub enum ExecutionCondition {
    Interval { interval: u64 },
    EventBased,
}

pub struct RunnerOptions {
    pub name: String,
    pub shutdown: Arc<Sender<bool>>,
    pub execution_condition: ExecutionCondition,
    pub strategy: Arc<dyn Strategy>,
    pub context_manager: Arc<dyn ContextManager>,
}

pub struct Runner {
    name: String,
    shutdown: Arc<Sender<bool>>,
    execution_condition: ExecutionCondition,
    strategy: Arc<dyn Strategy>,
    context_manager: Arc<dyn ContextManager>,
}

impl Runner {
    pub fn new(
        RunnerOptions {
            name,
            shutdown,
            execution_condition,
            strategy,
            context_manager,
        }: RunnerOptions,
    ) -> Self {
        Self {
            name,
            shutdown,
            execution_condition,
            strategy,
            context_manager,
        }
    }

    pub async fn run(&self) -> Result<(), RunnerError> {
        info!("[RUNNER-{}] Starting runner..", self.name);

        let res = match &self.execution_condition {
            ExecutionCondition::Interval { interval } => self.execute_on_interval(*interval).await,
            ExecutionCondition::EventBased => self.execute_on_event().await,
        };

        match res {
            Ok(()) => Ok(()),
            Err(e) => Err(e),
        }
    }

    async fn execute_on_interval(&self, interval: u64) -> Result<(), RunnerError> {
        let mut interval = time::interval(time::Duration::from_secs(interval));
        let mut shutdown_receiver = self.shutdown.subscribe();
        let mut shutdown_signal = false;

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let ctx = self.context_manager.build().await;
                    match self.strategy.execute(&ctx).await {
                        Ok(r) => {

                        },
                        Err(e) => {
                            info!("[RUNNER-{}] An error occurred while executing strategy: {:?}.", self.name, e);
                        }
                    }
                }
                _ = shutdown_receiver.recv() => {
                    shutdown_signal = true;
                }
            }
            if shutdown_signal {
                info!("[RUNNER-{}] Received shutdown signal, stopping.", self.name);
                break;
            }
        }

        Ok(())
    }

    async fn execute_on_event(&self) -> Result<(), RunnerError> {
        let mut shutdown_signal = false;
        let mut shutdown_receiver = self.shutdown.subscribe();
        let mut event_receiver = self.context_manager.subscribe();

        loop {
            tokio::select! {
                msg = event_receiver.recv() => {
                    match msg {
                        Ok(ctx) => {
                            match self.strategy.execute(&ctx).await {
                                Ok(r) => {

                                },
                                Err(e) => {
                                    info!("[RUNNER-{}] An error occurred while executing strategy: {:?}.", self.name, e);
                                }
                            }
                        },
                        Err(e) => {
                            warn!("[RUNNER-{}] There was an error receiving message from context manager: {:?}", self.name, e);
                        },
                    }
                }
                _ = shutdown_receiver.recv() => {
                    shutdown_signal = true;
                }
            }
            if shutdown_signal {
                info!("[RUNNER-{}] Received shutdown signal, stopping.", self.name);
                break;
            }
        }
        Ok(())
    }
}
