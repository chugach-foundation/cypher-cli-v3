use cypher_utils::contexts::{
    CacheContext, Fill, GenericEventQueue, GenericOpenOrders, GenericOrderBook, Order, OrderBook,
    UserContext,
};
use log::info;
use tokio::time::Instant;

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
        let now = Instant::now();
        let s = Self {
            orderbook: OrderBook {
                bids: orderbook.get_bids().await,
                asks: orderbook.get_asks().await,
            },
            open_orders: open_orders.get_open_orders(orderbook).await,
            fills: event_queue.get_fills().await,
        };
        let elapsed = now.elapsed();
        info!("[OPCTX] Build elapsed time: {:.2?}", elapsed.as_millis());
        s
    }
}

#[derive(Debug, Default, Clone)]
pub struct GlobalContext {
    /// The cache context.
    pub cache: CacheContext,
    /// The user context.
    pub user: UserContext,
}

impl GlobalContext {
    pub async fn build() -> Self {
        Self {
            cache: CacheContext::default(),
            user: UserContext::default(),
        }
    }
}

/// The execution context.
#[derive(Debug, Default, Clone)]
pub struct ExecutionContext {
    /// The operation context.
    pub operation: OperationContext,
    /// The global context.
    pub global: GlobalContext,
}

pub mod manager {
    use super::ExecutionContext;
    use async_trait::async_trait;
    use thiserror::Error;
    use tokio::sync::broadcast::{error::SendError, Receiver, Sender};

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
        /// Starts the [`ContextManager`], processing updates from the depending [`ContextBuilder`]s.
        async fn start(&self) -> Result<(), ContextManagerError>;

        /// Sends an update to context subscribers via it's own [`Sender`].
        async fn send(&self) -> Result<(), ContextManagerError>;

        /// Builds the [`ExecutionContext`] upon demand, representing the latest possible execution context.
        async fn build(&self) -> ExecutionContext;

        /// Subscribes to the [`ContextManager`].
        ///
        /// The given [`Receiver`] will periodically receive messages with the latest possible execution context.
        fn subscribe(&self) -> Receiver<ExecutionContext>;
    }
}

pub mod builder {
    use super::{ExecutionContext, GlobalContext, OperationContext};
    use async_trait::async_trait;
    use cypher_utils::accounts_cache::AccountState;
    use thiserror::Error;
    use tokio::sync::broadcast::{error::SendError, Receiver, Sender};

    #[derive(Error, Debug)]
    pub enum ContextBuilderError {
        #[error("Operation context send error: {:?}", self)]
        OperationContextSendError(SendError<OperationContext>),
        #[error("Global context send error: {:?}", self)]
        GlobalContextSendError(SendError<GlobalContext>),
        #[error("Process update error: {0}")]
        ProcessUpdateError(String),
    }

    /// A trait that represents shared functionality for context builders.
    ///
    /// This trait can be implemented to offer different contexts that are subsequently pluggable into various [`ContextManager`]s in order to build different execution environments.
    #[async_trait]
    pub trait ContextBuilder: Send + Sync {
        /// The output type for this [`ContextBuilder`]'s subscription events.
        type Output;

        /// Starts the [`ContextBuilder`],
        async fn start(&self) -> Result<(), ContextBuilderError>;

        /// Processes an [`AccountState`] update from the [`AccountsCache`].
        async fn process_update(
            &self,
            account_state: &AccountState,
        ) -> Result<(), ContextBuilderError>;

        /// Sends an update to context subscribers via it's own [`Sender`].
        async fn send(&self) -> Result<(), ContextBuilderError>;

        /// Subscribes to the [`ContextBuilder`].
        ///
        /// The given [`Receiver`] will periodically receive messages with the latest possible operation context.
        fn subscribe(&self) -> Receiver<Self::Output>;

        /// The symbol that this [`ContextBuilder`] represents.
        fn symbol(&self) -> String;
    }
}
