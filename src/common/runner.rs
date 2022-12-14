use log::{info, warn};
use std::{any::type_name, error, sync::Arc};
use thiserror::Error;
use tokio::{
    sync::broadcast::Sender,
    time::{self, *},
};

use super::{
    context::manager::ContextManager,
    orders::OrderManager,
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

pub struct RunnerOptions<CMO, CMGCI, CMOCI, OMO, CMOII> {
    pub name: String,
    pub shutdown: Arc<Sender<bool>>,
    pub execution_condition: ExecutionCondition,
    pub strategy: Arc<dyn Strategy<Input = CMO, Output = OMO>>,
    pub context_manager: Arc<
        dyn ContextManager<
            Output = CMO,
            GlobalContextInput = CMGCI,
            OperationContextInput = CMOCI,
            OracleInfoInput = CMOII,
        >,
    >,
}

pub struct Runner<CMO, CMGCI, CMOCI, OMO, CMOII> {
    name: String,
    shutdown: Arc<Sender<bool>>,
    execution_condition: ExecutionCondition,
    strategy: Arc<dyn Strategy<Input = CMO, Output = OMO>>,
    context_manager: Arc<
        dyn ContextManager<
            Output = CMO,
            GlobalContextInput = CMGCI,
            OperationContextInput = CMOCI,
            OracleInfoInput = CMOII,
        >,
    >,
}

impl<CMO, CMGCI, CMOCI, OMO, CMOII> Runner<CMO, CMGCI, CMOCI, OMO, CMOII>
where
    CMO: Clone + Send + Sync,
    CMGCI: Clone + Send + Sync,
    CMOCI: Clone + Send + Sync,
    OMO: Clone + Send + Sync,
    CMOII: Clone + Send + Sync,
{
    pub fn new(
        RunnerOptions {
            name,
            shutdown,
            execution_condition,
            strategy,
            context_manager,
        }: RunnerOptions<CMO, CMGCI, CMOCI, OMO, CMOII>,
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
        info!(
            "{} - [{}] Starting runner..",
            type_name::<Self>(),
            self.name
        );

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
                            info!("{} - [{}] An error occurred while executing strategy: {:?}.", type_name::<Self>(), self.name, e);
                        }
                    }
                }
                _ = shutdown_receiver.recv() => {
                    shutdown_signal = true;
                }
            }
            if shutdown_signal {
                info!(
                    "{} - [{}] Received shutdown signal, stopping.",
                    type_name::<Self>(),
                    self.name
                );
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
                                    info!("{} - [{}] An error occurred while executing strategy: {:?}.", type_name::<Self>(), self.name, e);
                                }
                            }
                        },
                        Err(e) => {
                            warn!("{} - [{}] There was an error receiving message from context manager: {:?}", type_name::<Self>(), self.name, e);
                        },
                    }
                }
                _ = shutdown_receiver.recv() => {
                    shutdown_signal = true;
                }
            }
            if shutdown_signal {
                info!(
                    "{} - [{}] Received shutdown signal, stopping.",
                    type_name::<Self>(),
                    self.name
                );
                break;
            }
        }
        Ok(())
    }
}
