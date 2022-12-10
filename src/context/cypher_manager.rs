use async_trait::async_trait;
use log::{info, warn};
use std::sync::Arc;
use tokio::sync::{
    broadcast::{channel, Receiver, Sender},
    RwLock,
};

use crate::common::context::{
    builder::ContextBuilder,
    manager::{ContextManager, ContextManagerError},
    ExecutionContext, GlobalContext, OperationContext,
};

use super::builders::global::GlobalContextBuilder;

pub struct CypherExecutionContextManager {
    global_context_builder: Arc<dyn ContextBuilder<Output = GlobalContext> + Send>,
    operation_context_builder: Arc<dyn ContextBuilder<Output = OperationContext> + Send>,
    global_context: RwLock<GlobalContext>,
    operation_context: RwLock<OperationContext>,
    update_sender: Arc<Sender<ExecutionContext>>,
    symbol: String,
}

impl CypherExecutionContextManager {
    pub fn new(
        global_context_builder: Arc<dyn ContextBuilder<Output = GlobalContext> + Send>,
        operation_context_builder: Arc<dyn ContextBuilder<Output = OperationContext> + Send>,
    ) -> Self {
        let symbol = operation_context_builder.symbol();
        Self {
            global_context_builder,
            operation_context_builder,
            global_context: RwLock::new(GlobalContext::default()),
            operation_context: RwLock::new(OperationContext::default()),
            update_sender: Arc::new(channel::<ExecutionContext>(1).0),
            symbol,
        }
    }
}

#[async_trait]
impl ContextManager for CypherExecutionContextManager {
    async fn start(&self) -> Result<(), ContextManagerError> {
        info!("[CECTXMGR-{}] Starting..", self.symbol);

        let mut g_ctx_update_receiver = self.global_context_builder.subscribe();
        let mut op_ctx_update_receiver = self.operation_context_builder.subscribe();

        loop {
            tokio::select! {
                g_ctx_update = g_ctx_update_receiver.recv() => {
                    if g_ctx_update.is_err() {
                        warn!("[CECTXMGR-{}] There was an error receiving global context update.", self.symbol);
                        continue;
                    } else {
                        let new_g_ctx = g_ctx_update.unwrap();
                        let mut g_ctx = self.global_context.write().await;
                        *g_ctx = new_g_ctx;
                        drop(g_ctx);
                        match self.send().await {
                            Ok(()) => (),
                            Err(e) => {
                                warn!("[CECTXMGR-{}] There was an error sending execution context update: {:?}", self.symbol, e.to_string());
                            }
                        };
                    }
                }
                op_ctx_update = op_ctx_update_receiver.recv() => {
                    if op_ctx_update.is_err() {
                        warn!("[CECTXMGR-{}] There was an error receiving operation context update.", self.symbol);
                        continue;
                    } else {
                        let new_op_ctx = op_ctx_update.unwrap();
                        let mut op_ctx = self.operation_context.write().await;
                        *op_ctx = new_op_ctx;
                        drop(op_ctx);
                        match self.send().await {
                            Ok(()) => (),
                            Err(e) => {
                                warn!("[CECTXMGR-{}] There was an error sending execution context update: {:?}", self.symbol, e.to_string());
                            }
                        };
                    }
                }
            }
        }

        Ok(())
    }

    async fn send(&self) -> Result<(), ContextManagerError> {
        let operation_context = self.operation_context.read().await;
        let global_context = self.global_context.read().await;

        info!(
            "[CECTXMGR-{}] Sending execution context update..",
            self.symbol
        );

        match self.update_sender.send(ExecutionContext {
            operation: operation_context.clone(),
            global: global_context.clone(),
        }) {
            Ok(_) => Ok(()),
            Err(e) => Err(ContextManagerError::SendError(e)),
        }
    }

    async fn build(&self) -> ExecutionContext {
        info!("[CECTXMGR-{}] Building execution context..", self.symbol);
        let operation_context = self.operation_context.read().await;
        let global_context = self.global_context.read().await;
        ExecutionContext {
            operation: operation_context.clone(),
            global: global_context.clone(),
        }
    }

    fn subscribe(&self) -> Receiver<ExecutionContext> {
        self.update_sender.subscribe()
    }
}
