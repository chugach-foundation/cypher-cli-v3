use async_trait::async_trait;
use fixed::types::I80F48;
use log::{info};
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;
use tokio::sync::{
    broadcast::{channel, Receiver, Sender},
    RwLock, RwLockWriteGuard,
};

use crate::common::{
    context::{
        builder::ContextBuilder,
        manager::{ContextManager, ContextManagerError},
        ExecutionContext, GlobalContext, OperationContext,
    },
    oracle::{OracleInfo, OracleProvider},
};



pub struct CypherExecutionContextManager {
    global_context_builder: Arc<dyn ContextBuilder<Output = GlobalContext> + Send>,
    operation_context_builder: Arc<dyn ContextBuilder<Output = OperationContext> + Send>,
    oracle_provider: Arc<dyn OracleProvider<Input = GlobalContext> + Send>,
    global_context: RwLock<GlobalContext>,
    operation_context: RwLock<OperationContext>,
    oracle_info: RwLock<OracleInfo>,
    update_sender: Arc<Sender<ExecutionContext>>,
    shutdown_sender: Arc<Sender<bool>>,
    symbol: String,
}

impl CypherExecutionContextManager {
    pub fn new(
        shutdown_sender: Arc<Sender<bool>>,
        global_context_builder: Arc<dyn ContextBuilder<Output = GlobalContext> + Send>,
        operation_context_builder: Arc<dyn ContextBuilder<Output = OperationContext> + Send>,
        oracle_provider: Arc<dyn OracleProvider<Input = GlobalContext> + Send>,
    ) -> Self {
        let symbol = operation_context_builder.symbol().to_string();
        Self {
            global_context_builder,
            operation_context_builder,
            oracle_provider,
            global_context: RwLock::new(GlobalContext::default()),
            operation_context: RwLock::new(OperationContext::default()),
            oracle_info: RwLock::new(OracleInfo::default()),
            update_sender: Arc::new(channel::<ExecutionContext>(50).0),
            shutdown_sender,
            symbol,
        }
    }
}

#[async_trait]
impl ContextManager for CypherExecutionContextManager {
    type Output = ExecutionContext;
    type GlobalContextInput = GlobalContext;
    type OperationContextInput = OperationContext;
    type OracleInfoInput = OracleInfo;

    async fn operation_context_writer(&self) -> RwLockWriteGuard<OperationContext> {
        self.operation_context.write().await
    }

    async fn global_context_writer(&self) -> RwLockWriteGuard<GlobalContext> {
        self.global_context.write().await
    }

    async fn oracle_info_writer(&self) -> RwLockWriteGuard<OracleInfo> {
        self.oracle_info.write().await
    }

    fn global_context_receiver(&self) -> Receiver<GlobalContext> {
        self.global_context_builder.subscribe()
    }

    fn operation_context_receiver(&self) -> Receiver<OperationContext> {
        self.operation_context_builder.subscribe()
    }

    fn oracle_info_receiver(&self) -> Receiver<OracleInfo> {
        self.oracle_provider.subscribe()
    }

    fn shutdown_receiver(&self) -> Receiver<bool> {
        self.shutdown_sender.subscribe()
    }

    async fn send(&self) -> Result<(), ContextManagerError> {
        let operation_context = self.operation_context.read().await;
        let global_context = self.global_context.read().await;
        let oracle_info = self.oracle_info.read().await;

        // sanity checks
        // let's hold off on sending an execution context update until
        // 1. oracle price has been received
        if oracle_info.price == I80F48::ZERO {
            info!(
                "[{}] Oracle price unset - Awaiting to send execution context..",
                self.symbol
            );
            return Ok(());
        }

        // 2. account context exist
        if global_context.user.account_ctx.state.authority == Pubkey::default() {
            info!(
                "[{}] User account unset - Awaiting to send execution context..",
                self.symbol
            );
            return Ok(());
        }

        // 3. sub accounts have been loaded
        if global_context.user.sub_account_ctxs.is_empty() {
            info!(
                "[{}] User sub accounts unset - Awaiting to send execution context..",
                self.symbol
            );
            return Ok(());
        }

        info!("[{}] Sending execution context update..", self.symbol);

        match self.update_sender.send(ExecutionContext {
            operation: operation_context.clone(),
            global: global_context.clone(),
            oracle_info: oracle_info.clone(),
        }) {
            Ok(_) => Ok(()),
            Err(e) => Err(ContextManagerError::SendError(e)),
        }
    }

    async fn build(&self) -> ExecutionContext {
        info!("[{}] Building execution context..", self.symbol);
        let operation_context = self.operation_context.read().await;
        let global_context = self.global_context.read().await;
        let oracle_info = self.oracle_info.read().await;
        ExecutionContext {
            operation: operation_context.clone(),
            global: global_context.clone(),
            oracle_info: oracle_info.clone(),
        }
    }

    fn sender(&self) -> Arc<Sender<ExecutionContext>> {
        self.update_sender.clone()
    }

    fn subscribe(&self) -> Receiver<ExecutionContext> {
        self.update_sender.subscribe()
    }

    fn symbol(&self) -> &str {
        self.symbol.as_str()
    }
}
