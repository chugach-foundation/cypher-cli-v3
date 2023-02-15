use super::{
    info::{Accounts, MarketMetadata, UserInfo},
    oracle::OracleInfo,
};
use cypher_utils::contexts::{
    CacheContext, Fill, GenericEventQueue, GenericOpenOrders, GenericOrderBook, Order, OrderBook,
    UserContext,
};

use solana_sdk::pubkey::Pubkey;

/// Represents information about a certain operating context.
/// This structure makes it easier to build out the remaining components used to operate.
#[derive(Default, Clone)]
pub struct ContextInfo {
    /// The symbol of the market.
    pub symbol: String,
    /// The user accounts necessary to operate on the market.
    pub user_accounts: UserInfo,
    /// Metadata related to the market.
    pub market_metadata: MarketMetadata,
    /// The market accounts necessary to operate on the market.
    pub context_accounts: Accounts,
}

impl ContextInfo {
    pub fn context_accounts(&self) -> Vec<Pubkey> {
        match &self.context_accounts {
            Accounts::Futures(f) => f.to_accounts_vec(),
            Accounts::Perpetuals(p) => p.to_accounts_vec(),
            Accounts::Spot(s) => s.to_accounts_vec(),
        }
    }
}

/// Represents the operation context for the market maker at a given point in time.
///
/// This structure stores the book and available open orders.
#[derive(Debug, Default, Clone)]
pub struct OperationContext {
    /// The latest order book snapshot.
    pub orderbook: OrderBook,
    /// Orders open in the Cypher or Serum Orders Accounts.
    pub open_orders: Vec<Order>,
    /// Recent fills that have occurred.
    pub fills: Vec<Fill>,
}

impl OperationContext {
    pub async fn build(
        orderbook: &dyn GenericOrderBook,
        open_orders: &dyn GenericOpenOrders,
        event_queue: &dyn GenericEventQueue,
    ) -> Self {
        Self {
            orderbook: OrderBook {
                bids: orderbook.get_bids(),
                asks: orderbook.get_asks(),
            },
            open_orders: open_orders.get_open_orders(orderbook),
            fills: event_queue.get_fills(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct GlobalContext {
    /// The cache context.
    pub cache: CacheContext,
    /// The user context.
    pub user: UserContext,
}

/// The execution context.
#[derive(Debug, Default, Clone)]
pub struct ExecutionContext {
    /// The operation context.
    pub operation: OperationContext,
    /// The global context.
    pub global: GlobalContext,
    /// The oracle information for this specific context.
    pub oracle_info: OracleInfo,
}

pub trait OrdersContext {
    /// Gets the open orders.
    fn get_open_orders(&self) -> &[Order];

    /// Gets the orderbook.
    fn get_orderbook(&self) -> &OrderBook;
}

impl OrdersContext for ExecutionContext {
    fn get_open_orders(&self) -> &[Order] {
        &self.operation.open_orders
    }

    fn get_orderbook(&self) -> &OrderBook {
        &self.operation.orderbook
    }
}

pub mod manager {
    use super::ExecutionContext;
    use async_trait::async_trait;
    use log::{info, warn};
    use std::sync::Arc;
    use thiserror::Error;
    use tokio::sync::{
        broadcast::{error::SendError, Receiver, Sender},
        RwLockWriteGuard,
    };

    #[derive(Error, Debug)]
    pub enum ContextManagerError {
        #[error("Send error: {:?}", self)]
        SendError(SendError<ExecutionContext>),
    }

    /// A trait that represents shared functionality for context managers.
    ///
    /// The goal behind this trait is to have an entity that is responsible for managing an execution environment,
    /// this should encompass global states (i.e. accounts, sub accounts), external feeds (i.e. oracles, signals etc), and the execution environment (i.e. orderbook data, open orders etc).
    ///
    /// This trait can be implemented to offer different execution environments for strategies.
    #[async_trait]
    pub trait ContextManager: Send + Sync {
        /// The output type for this [`ContextManager`]'s subscription events.
        type Output: Clone + Send + Sync;

        /// The input type for this [`ContextManager`]'s global context.
        type GlobalContextInput: Clone + Send + Sync;

        /// The input type for this [`ContextManager`]'s operation context.
        type OperationContextInput: Clone + Send + Sync;

        /// The input type for this [`ContextManager`]'s oracle info.
        type OracleInfoInput: Clone + Send + Sync;

        /// Gets the writer lock for the [`Self::GlobalContextInput`].
        async fn global_context_writer(&self) -> RwLockWriteGuard<Self::GlobalContextInput>;

        /// Gets the writer lock for the [`Self::OperationContextInput`].
        async fn operation_context_writer(&self) -> RwLockWriteGuard<Self::OperationContextInput>;

        /// Gets the writer lock for the [`Self::OracleInfoInput`].
        async fn oracle_info_writer(&self) -> RwLockWriteGuard<Self::OracleInfoInput>;

        /// Gets the [`Self::GlobalContextInput`] [`Receiver`].
        fn global_context_receiver(&self) -> Receiver<Self::GlobalContextInput>;

        /// Gets the [`Self::OperationContextInput`] [`Receiver`].
        fn operation_context_receiver(&self) -> Receiver<Self::OperationContextInput>;

        /// Gets the [`Self::OracleInfoInput`] [`Receiver`].
        fn oracle_info_receiver(&self) -> Receiver<Self::OracleInfoInput>;

        /// Gets the shutdown [`Receiver`].
        fn shutdown_receiver(&self) -> Receiver<bool>;

        /// Starts the [`ContextManager`],
        async fn start(&self) -> Result<(), ContextManagerError> {
            let mut g_ctx_update_receiver = self.global_context_receiver();
            let mut op_ctx_update_receiver = self.operation_context_receiver();
            let mut oracle_update_receiver = self.oracle_info_receiver();
            let mut shutdown_receiver = self.shutdown_receiver();
            let symbol = self.symbol();

            info!("[{}] Starting..", symbol);

            loop {
                tokio::select! {
                    g_ctx_update = g_ctx_update_receiver.recv() => {
                        match g_ctx_update {
                            Ok(update) => {
                                let mut g_ctx = self.global_context_writer().await;
                                *g_ctx = update;
                                drop(g_ctx);
                                match self.send().await {
                                    Ok(()) => (),
                                    Err(e) => {
                                        warn!("[{}] There was an error sending execution context update: {:?}", symbol, e.to_string());
                                    }
                                };
                            },
                            Err(_) => {
                                warn!("[{}] There was an error receiving global context update.", symbol);
                                continue;
                            }
                        }
                    }
                    op_ctx_update = op_ctx_update_receiver.recv() => {
                        match op_ctx_update {
                            Ok(update) => {
                                let mut op_ctx = self.operation_context_writer().await;
                                *op_ctx = update;
                                drop(op_ctx);
                                match self.send().await {
                                    Ok(()) => (),
                                    Err(e) => {
                                        warn!("[{}] There was an error sending execution context update: {:?}", symbol, e.to_string());
                                    }
                                };
                            },
                            Err(_) => {
                                warn!("[{}] There was an error receiving operation context update.", symbol);
                                continue;
                            }
                        }
                    }
                    oracle_update = oracle_update_receiver.recv() => {
                        match oracle_update {
                            Ok(update) => {
                                let mut oracle_info = self.oracle_info_writer().await;
                                *oracle_info = update;
                                drop(oracle_info);
                                match self.send().await {
                                    Ok(()) => (),
                                    Err(e) => {
                                        warn!("[{}] There was an error sending execution context update: {:?}", symbol, e.to_string());
                                    }
                                };
                            },
                            Err(_) => {
                                warn!("[{}] There was an error receiving oracle update.", symbol);
                                continue;
                            }
                        }
                    }
                    _ = shutdown_receiver.recv() => {
                        info!("[{}] Shutdown signal received, stopping..", symbol);
                        break;
                    }
                }
            }

            Ok(())
        }

        /// Sends an update to context subscribers via it's own [`Sender`].
        async fn send(&self) -> Result<(), ContextManagerError>;

        /// Builds the [`Self::Output`] upon demand, representing the latest possible execution context.
        async fn build(&self) -> Self::Output;

        /// Gets a clone of the [`Sender`].
        fn sender(&self) -> Arc<Sender<Self::Output>>;

        /// Subscribes to the [`ContextManager`].
        ///
        /// The given [`Receiver`] will periodically receive messages with the latest possible execution context.
        fn subscribe(&self) -> Receiver<Self::Output>;

        /// The symbol that this [`ContextManager`] represents.
        fn symbol(&self) -> &str;
    }
}

pub mod builder {
    use super::{GlobalContext, OperationContext};
    use async_trait::async_trait;
    use cypher_utils::accounts_cache::AccountState;
    use log::{info, warn};
    use solana_sdk::pubkey::Pubkey;
    use std::sync::Arc;
    use thiserror::Error;
    use tokio::sync::broadcast::{error::SendError, Receiver, Sender};

    #[derive(Error, Debug)]
    pub enum ContextBuilderError {
        #[error("Operation context send error: {:?}", self)]
        OperationContextSendError(SendError<OperationContext>),
        #[error("Global context send error: {:?}", self)]
        GlobalContextSendError(SendError<GlobalContext>),
        #[error("Unrecognized account error: {0}")]
        UnrecognizedAccount(Pubkey),
    }

    /// A trait that represents shared functionality for context builders.
    ///
    /// This trait can be implemented to offer different contexts that are subsequently pluggable into various [`ContextManager`]s in order to build different execution environments.
    #[async_trait]
    pub trait ContextBuilder: Send + Sync {
        /// The output type for this [`ContextBuilder`]'s subscription events.
        type Output;

        /// Gets the [`AccountState`] [`Receiver`].
        async fn cache_receiver(&self) -> Receiver<AccountState>;

        /// Gets the shutdown [`Receiver`].
        fn shutdown_receiver(&self) -> Receiver<bool>;

        /// Starts the [`ContextBuilder`],
        async fn start(&self) -> Result<(), ContextBuilderError> {
            let mut cache_receiver = self.cache_receiver().await;
            let mut shutdown_receiver = self.shutdown_receiver();
            let symbol = self.symbol();

            info!("[{}] Starting context builder..", symbol);

            loop {
                tokio::select! {
                    account_state_update = cache_receiver.recv() => {
                        match account_state_update {
                            Ok(account_state) => {
                                match self.process_update(&account_state).await {
                                    Ok(()) => {
                                        match self.send().await {
                                            Ok(()) => (),
                                            Err(e) => {
                                                warn!("[{}] There was an error sending context update: {:?}", symbol, e.to_string());
                                            }
                                        };
                                    },
                                    Err(_) => () // let's just ignore this for now because we are filtering accounts we don't need through errors
                                };
                            },
                            Err(_e) => {
                                warn!("[{}] There was an error receiving account state update.", symbol);
                            }
                        }
                    }
                    _ = shutdown_receiver.recv() => {
                        info!("[{}] Shutdown signal received, stopping..", symbol);
                        break;
                    }
                }
            }

            Ok(())
        }

        /// Processes an [`AccountState`] update from the [`AccountsCache`].
        async fn process_update(
            &self,
            account_state: &AccountState,
        ) -> Result<(), ContextBuilderError>;

        /// Sends an update to context subscribers via it's own [`Sender`].
        async fn send(&self) -> Result<(), ContextBuilderError>;

        /// Gets a clone of the [`Sender`].
        fn sender(&self) -> Arc<Sender<Self::Output>>;

        /// Subscribes to the [`ContextBuilder`].
        ///
        /// The given [`Receiver`] will periodically receive messages with the latest possible operation context.
        fn subscribe(&self) -> Receiver<Self::Output>;

        /// The symbol that this [`ContextBuilder`] represents.
        fn symbol(&self) -> &str;
    }
}
