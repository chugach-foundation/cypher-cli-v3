use anchor_lang::{Owner, ZeroCopy};
use async_trait::async_trait;
use cypher_client::{Market, Side};
use cypher_utils::{
    accounts_cache::{AccountState, AccountsCache},
    contexts::{
        AgnosticEventQueueContext, AgnosticOpenOrdersContext, AgnosticOrderBookContext,
        ContextError, MarketContext, PoolContext,
    },
};
use log::{info, warn};
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;
use tokio::sync::{
    broadcast::{channel, Receiver, Sender},
    RwLock,
};

use crate::common::context::{
    builder::{ContextBuilder, ContextBuilderError},
    OperationContext,
};

/// The context state used for a derivatives market operation.
#[derive(Default)]
pub struct DerivativeContextState<T> {
    /// The quote pool context.
    pub quote_pool: PoolContext,
    /// The market context.
    pub market: MarketContext<T>,
    /// The AOB event queue context.
    pub event_queue: AgnosticEventQueueContext,
    /// The AOB orderbook context.
    pub orderbook: AgnosticOrderBookContext,
    /// The Cypher open orders context.
    pub open_orders: AgnosticOpenOrdersContext,
}

impl<T> DerivativeContextState<T>
where
    T: Default + Market + ZeroCopy + Owner + Send + Sync,
{
    fn update_quote_pool(&mut self, pool: &Pubkey, data: &[u8]) {
        self.quote_pool.reload_from_account_data(data);
    }

    fn update_quote_pool_node(&mut self, pool_node: &Pubkey, data: &[u8]) {
        self.quote_pool
            .reload_pool_node_from_account_data(pool_node, data);
    }

    fn update_market(&mut self, market: &Pubkey, data: &[u8]) {
        self.market.reload_from_account_data(data);
    }

    fn update_event_queue(&mut self, market: &Pubkey, event_queue: &Pubkey, data: &[u8]) {
        self.event_queue.reload_from_account_data(data);
    }

    fn update_orderbook(
        &mut self,
        market: &Pubkey,
        bids: &Pubkey,
        asks: &Pubkey,
        market_state: &dyn Market,
        data: &[u8],
        side: Side,
    ) {
        self.orderbook
            .reload_from_account_data(market_state, data, side);
    }

    fn update_open_orders(&mut self, account: &Pubkey, data: &[u8]) {
        self.open_orders.reload_from_account_data(data);
    }
}

/// Builds and maintains an updated operation context for the market maker at all times.
///
/// Receives updates from the [AccountsCache] about all necessary accounts to operate on a given market.
pub struct DerivativeContextBuilder<T> {
    accounts_cache: Arc<AccountsCache>,
    market_state: T,
    market: Pubkey,
    event_queue: Pubkey,
    bids: Pubkey,
    asks: Pubkey,
    open_orders: Pubkey,
    quote_pool: Pubkey,
    quote_pool_nodes: Vec<Pubkey>,
    state: RwLock<DerivativeContextState<T>>,
    shutdown_sender: Arc<Sender<bool>>,
    update_sender: Arc<Sender<OperationContext>>,
    symbol: String,
}

impl<T> DerivativeContextBuilder<T>
where
    T: Default + Market + ZeroCopy + Owner + Send + Sync,
{
    /// Creates a new [`DerivativeContextBuilder<T>`].
    pub fn new(
        accounts_cache: Arc<AccountsCache>,
        shutdown_sender: Arc<Sender<bool>>,
        market_state: T,
        market: Pubkey,
        event_queue: Pubkey,
        bids: Pubkey,
        asks: Pubkey,
        open_orders: Pubkey,
        quote_pool: Pubkey,
        quote_pool_nodes: Vec<Pubkey>,
        symbol: String,
    ) -> Self {
        Self {
            accounts_cache,
            shutdown_sender,
            market_state,
            market,
            event_queue,
            bids,
            asks,
            open_orders,
            quote_pool,
            quote_pool_nodes,
            state: RwLock::new(DerivativeContextState::<T>::default()),
            update_sender: Arc::new(channel::<OperationContext>(50).0),
            symbol,
        }
    }
}

#[async_trait]
impl<T> ContextBuilder for DerivativeContextBuilder<T>
where
    T: Default + Market + ZeroCopy + Owner + Send + Sync,
{
    type Output = OperationContext;

    async fn cache_receiver(&self) -> Receiver<AccountState> {
        let mut accounts = vec![
            self.market,
            self.event_queue,
            self.bids,
            self.asks,
            self.open_orders,
            self.quote_pool,
        ];
        accounts.extend(self.quote_pool_nodes.clone());
        self.accounts_cache.subscribe(&accounts).await
    }

    fn shutdown_receiver(&self) -> Receiver<bool> {
        self.shutdown_sender.subscribe()
    }

    async fn send(&self) -> Result<(), ContextBuilderError> {
        let state = self.state.read().await;
        let ctx =
            OperationContext::build(&state.orderbook, &state.open_orders, &state.event_queue).await;
        match self.update_sender.send(ctx) {
            Ok(_) => Ok(()),
            Err(e) => Err(ContextBuilderError::OperationContextSendError(e)),
        }
    }

    /// Returns the process update of this [`DerivativeContextBuilder<T>`].
    async fn process_update(
        &self,
        account_state: &AccountState,
    ) -> Result<(), ContextBuilderError> {
        // check if this account is the quote pool for this market
        if account_state.account == self.quote_pool {
            let mut state = self.state.write().await;
            state.update_quote_pool(&self.quote_pool, &account_state.data);
            info!(
                "[{}] Sucessfully processed quote pool account update.",
                self.symbol
            );
            return Ok(());
        }

        // check if it is one of the quote pool nodes
        if self.quote_pool_nodes.contains(&account_state.account) {
            let mut state = self.state.write().await;
            state.update_quote_pool_node(&account_state.account, &account_state.data);
            info!(
                "[{}] Sucessfully processed quote pool node account update.",
                self.symbol
            );
            return Ok(());
        }

        // check if this account is the market
        if account_state.account == self.market {
            let mut state = self.state.write().await;
            state.update_market(&self.market, &account_state.data);
            info!(
                "[{}] Sucessfully processed derivative market account update.",
                self.symbol
            );
            return Ok(());
        }

        // check if this account is the event queue
        if account_state.account == self.event_queue {
            let mut state = self.state.write().await;
            state.update_event_queue(&self.market, &self.event_queue, &account_state.data);
            info!(
                "[{}] Sucessfully processed derivative market event queue account update.",
                self.symbol
            );
            return Ok(());
        }

        // check if this account is the bid side of the book
        if account_state.account == self.bids {
            let mut state = self.state.write().await;
            state.update_orderbook(
                &self.market,
                &self.bids,
                &self.asks,
                &self.market_state,
                &account_state.data,
                Side::Bid,
            );
            info!(
                "[{}] Sucessfully processed derivative market bids account update.",
                self.symbol
            );
            return Ok(());
        }

        // check if this account is the ask side of the book
        if account_state.account == self.asks {
            let mut state = self.state.write().await;
            state.update_orderbook(
                &self.market,
                &self.bids,
                &self.asks,
                &self.market_state,
                &account_state.data,
                Side::Ask,
            );
            info!(
                "[{}] Sucessfully processed derivative market asks account update.",
                self.symbol
            );
            return Ok(());
        }

        // check if this account is the open orders
        if account_state.account == self.open_orders {
            let mut state = self.state.write().await;
            state.update_open_orders(&self.open_orders, &account_state.data);
            info!(
                "[{}] Sucessfully processed derivative market open orders account update.",
                self.symbol
            );
            return Ok(());
        }

        Err(ContextBuilderError::UnrecognizedAccount(
            account_state.account,
        ))
    }

    fn sender(&self) -> Arc<Sender<OperationContext>> {
        self.update_sender.clone()
    }

    fn subscribe(&self) -> Receiver<OperationContext> {
        self.update_sender.subscribe()
    }

    fn symbol(&self) -> &str {
        self.symbol.as_str()
    }
}
