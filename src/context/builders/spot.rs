use anchor_spl::dex::serum_dex::state::MarketState;
use async_trait::async_trait;
use cypher_client::Side;
use cypher_utils::{
    accounts_cache::{AccountState, AccountsCache},
    contexts::{
        ContextError, PoolContext, SerumEventQueueContext, SerumOpenOrdersContext,
        SerumOrderBookContext,
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

/// The context state used for a spot market operation.
#[derive(Default)]
pub struct SpotContextState {
    /// The pool context.
    pub pool: PoolContext,
    /// The Serum event queue context.
    pub event_queue: SerumEventQueueContext,
    /// The Serum orderbook context.
    pub orderbook: SerumOrderBookContext,
    /// The Serum open orders context.
    pub open_orders: SerumOpenOrdersContext,
}

impl SpotContextState {
    fn update_pool(&mut self, market: &Pubkey, data: &[u8]) -> Result<(), ContextError> {
        self.pool = match PoolContext::from_account_data(data, market) {
            Ok(p) => p,
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
            match SerumEventQueueContext::from_account_data(market, event_queue, data) {
                Ok(eq) => eq,
                Err(e) => return Err(e),
            };
        Ok(())
    }

    async fn update_orderbook(&mut self, market_state: &MarketState, data: &[u8], side: Side) {
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
pub struct SpotContextBuilder {
    accounts_cache: Arc<AccountsCache>,
    market_state: MarketState,
    market: Pubkey,
    event_queue: Pubkey,
    bids: Pubkey,
    asks: Pubkey,
    open_orders: Pubkey,
    state: RwLock<SpotContextState>,
    update_sender: Arc<Sender<OperationContext>>,
    symbol: String,
}

impl SpotContextBuilder {
    /// Creates a new [`SpotContextBuilder`].
    pub fn new(
        accounts_cache: Arc<AccountsCache>,
        market_state: MarketState,
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
            state: RwLock::new(SpotContextState::default()),
            update_sender: Arc::new(channel::<OperationContext>(1).0),
            symbol,
        }
    }
}

#[async_trait]
impl ContextBuilder for SpotContextBuilder {
    type Output = OperationContext;

    async fn start(&self) -> Result<(), ContextBuilderError> {
        let mut sub = self.accounts_cache.subscribe();

        loop {
            tokio::select! {
                account_state_update = sub.recv() => {
                    if account_state_update.is_err() {
                        warn!("[SPOTCTX-BLDR] There was an error receiving account state update.");
                        continue;
                    } else {
                        let account_state = account_state_update.unwrap();
                        match self.process_update(&account_state).await {
                            Ok(()) => (),
                            Err(e) => {
                                warn!("[SPOTCTX-BLDR] An error occurred while processing account update: {:?}", e);
                            }
                        };
                        match self.send().await {
                            Ok(()) => (),
                            Err(e) => {
                                warn!("[SPOTCTX-BLDR] There was an error sending operation context update: {:?}", e.to_string());
                            }
                        };
                    }
                }
            }
        }

        Ok(())
    }

    /// Returns the process update of this [`SpotContextBuilder`].
    async fn process_update(
        &self,
        account_state: &AccountState,
    ) -> Result<(), ContextBuilderError> {
        // check if this account is the market
        if account_state.account == self.market {
            let mut state = self.state.write().await;
            match state.update_pool(&self.market, &account_state.data) {
                Ok(()) => {
                    info!("[SPOTCTX-BLDR] Sucessfully processed spot pool account update.");
                }
                Err(e) => {
                    return Err(ContextBuilderError::ProcessUpdateError(
                        "pool account".to_string(),
                    ))
                }
            }
        }

        // check if this account is the event queue
        if account_state.account == self.event_queue {
            let mut state = self.state.write().await;
            match state.update_event_queue(&self.market, &self.event_queue, &account_state.data) {
                Ok(()) => {
                    info!("[SPOTCTX-BLDR] Sucessfully processed spot market event queue account update.");
                }
                Err(e) => {
                    return Err(ContextBuilderError::ProcessUpdateError(
                        "event queue".to_string(),
                    ))
                }
            }
        }

        // check if this account is the bid side of the book
        if account_state.account == self.bids {
            let mut state = self.state.write().await;
            state
                .update_orderbook(&self.market_state, &account_state.data, Side::Bid)
                .await;
            info!("[SPOTCTX-BLDR] Sucessfully processed spot market bids account update.");
        }

        // check if this account is the ask side of the book
        if account_state.account == self.asks {
            let mut state = self.state.write().await;
            state
                .update_orderbook(&self.market_state, &account_state.data, Side::Ask)
                .await;
            info!("[SPOTCTX-BLDR] Sucessfully processed spot market asks account update.");
        }

        // check if this account is the open orders
        if account_state.account == self.open_orders {
            let mut state = self.state.write().await;
            state
                .update_open_orders(&self.open_orders, &account_state.data)
                .await;
            info!("[SPOTCTX-BLDR] Sucessfully processed spot market open orders account update.");
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

    fn subscribe(&self) -> Receiver<OperationContext> {
        self.update_sender.subscribe()
    }

    fn symbol(&self) -> String {
        self.symbol.to_string()
    }
}
