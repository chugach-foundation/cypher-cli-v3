use anchor_lang::{Owner, ZeroCopy};
use async_trait::async_trait;
use cypher_client::{Market, Side};
use cypher_utils::{
    accounts_cache::{AccountState, AccountsCache},
    contexts::{
        AgnosticEventQueueContext, AgnosticOpenOrdersContext, AgnosticOrderBookContext,
        ContextError, MarketContext,
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
    fn update_market(&mut self, market: &Pubkey, data: &[u8]) -> Result<(), ContextError> {
        self.market = match MarketContext::<T>::from_account_data(data, market) {
            Ok(m) => m,
            Err(e) => {
                return Err(e);
            }
        };
        Ok(())
    }

    fn update_event_queue(
        &mut self,
        market: &Pubkey,
        event_queue: &Pubkey,
        data: &[u8],
    ) -> Result<(), ContextError> {
        self.event_queue =
            match AgnosticEventQueueContext::from_account_data(market, event_queue, data) {
                Ok(eq) => eq,
                Err(e) => {
                    return Err(e);
                }
            };
        Ok(())
    }

    async fn update_orderbook(&mut self, market_state: &dyn Market, data: &[u8], side: Side) {
        self.orderbook
            .reload_from_account_data(market_state, data, side)
            .await
    }

    async fn update_open_orders(&mut self, account: &Pubkey, data: &[u8]) {
        self.open_orders.reload_from_account_data(data).await;
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
    state: RwLock<DerivativeContextState<T>>,
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
        market_state: T,
        market: Pubkey,
        event_queue: Pubkey,
        bids: Pubkey,
        asks: Pubkey,
        open_orders: Pubkey,
        symbol: String,
    ) -> Self {
        Self {
            accounts_cache,
            market_state,
            market,
            event_queue,
            bids,
            asks,
            open_orders,
            state: RwLock::new(DerivativeContextState::<T>::default()),
            update_sender: Arc::new(channel::<OperationContext>(1).0),
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

    async fn start(&self) -> Result<(), ContextBuilderError> {
        let mut sub = self.accounts_cache.subscribe();

        loop {
            tokio::select! {
                account_state_update = sub.recv() => {
                    if account_state_update.is_err() {
                        warn!("[DRVCTX-BLDR-{}] There was an error receiving account state update.", self.symbol);
                        continue;
                    } else {
                        let account_state = account_state_update.unwrap();
                        match self.process_update(&account_state).await {
                            Ok(()) => (),
                            Err(e) => {
                                warn!("[DRVCTX-BLDR-{}] An error occurred while processing account update: {:?}", self.symbol, e);
                            }
                        };
                        match self.send().await {
                            Ok(()) => (),
                            Err(e) => {
                                warn!("[DRVCTX-BLDR-{}] There was an error sending operation context update: {:?}", self.symbol, e.to_string());
                            }
                        };
                    }
                }
            }
        }

        Ok(())
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
        // check if this account is the market
        if account_state.account == self.market {
            let mut state = self.state.write().await;
            match state.update_market(&self.market, &account_state.data) {
                Ok(()) => {
                    info!(
                        "[DRVCTX-BLDR-{}] Sucessfully processed derivative market account update.",
                        self.symbol
                    );
                }
                Err(e) => {
                    return Err(ContextBuilderError::ProcessUpdateError(
                        "market account".to_string(),
                    ))
                }
            }
        }

        // check if this account is the event queue
        if account_state.account == self.event_queue {
            let mut state = self.state.write().await;
            match state.update_event_queue(&self.market, &self.event_queue, &account_state.data) {
                Ok(()) => {
                    info!("[DRVCTX-BLDR-{}] Sucessfully processed derivative market event queue account update.", self.symbol);
                }
                Err(e) => {
                    return Err(ContextBuilderError::ProcessUpdateError(
                        "event queue".to_string(),
                    ))
                }
            };
        }

        // check if this account is the bid side of the book
        if account_state.account == self.bids {
            let mut state = self.state.write().await;
            state
                .update_orderbook(&self.market_state, &account_state.data, Side::Bid)
                .await;
            info!(
                "[DRVCTX-BLDR-{}] Sucessfully processed derivative market bids account update.",
                self.symbol
            );
        }

        // check if this account is the ask side of the book
        if account_state.account == self.asks {
            let mut state = self.state.write().await;
            state
                .update_orderbook(&self.market_state, &account_state.data, Side::Ask)
                .await;
            info!(
                "[DRVCTX-BLDR-{}] Sucessfully processed derivative market asks account update.",
                self.symbol
            );
        }

        // check if this account is the open orders
        if account_state.account == self.open_orders {
            let mut state = self.state.write().await;
            state
                .update_open_orders(&self.open_orders, &account_state.data)
                .await;
            info!(
                "[DRVCTX-BLDR-{}] Sucessfully processed derivative market open orders account update.", self.symbol
            );
        }

        Ok(())
    }

    fn subscribe(&self) -> Receiver<OperationContext> {
        self.update_sender.subscribe()
    }

    fn symbol(&self) -> String {
        self.symbol.to_string()
    }
}
