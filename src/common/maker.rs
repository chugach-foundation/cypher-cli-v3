use async_trait::async_trait;
use cypher_client::Side;
use cypher_utils::contexts::Order;
use fixed::types::I80F48;
use log::{info, warn};
use solana_sdk::instruction::Instruction;
use std::{
    any::type_name,
    ops::{Add, Div, Mul, Sub},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::sync::{
    broadcast::{error::SendError, Receiver, Sender},
    RwLockReadGuard, RwLockWriteGuard,
};

use crate::market_maker::constants::BPS_UNIT;

use super::{
    context::{ExecutionContext, OrdersContext},
    info::{Accounts, UserInfo},
    inventory::{InventoryManager, QuoteVolumes, SpreadInfo},
    orders::{
        Action, CandidateCancel, CandidatePlacement, ManagedOrder, OrderManager, OrderManagerError,
    },
};

/// Represents the result of a maker's pulse.
#[derive(Default, Debug, Clone)]
pub struct MakerPulseResult {
    /// Number of new orders submitted.
    pub num_new_orders: usize,
    /// Number of cancelled orders.
    pub num_cancelled_orders: usize,
}

#[derive(Debug, Error)]
pub enum MakerError {
    #[error(transparent)]
    OrderManagerError(#[from] OrderManagerError),
    #[error("Action send error: {:?}", self)]
    ActionSendError(SendError<Action>),
}

/// Defines shared functionality that different makers should implement
#[async_trait]
pub trait Maker: Send + Sync {
    /// The input type for the maker.
    type Input: Clone + Send + Sync;

    /// The input type for the inventory manager.
    type InventoryManagerInput: Clone + Send + Sync;

    /// Gets the inventory manager for the maker.
    fn inventory_manager(&self) -> Arc<dyn InventoryManager<Input = Self::InventoryManagerInput>>;

    /// Gets the [`Receiver`] for the respective [`Self::Input`] data type.
    fn context_receiver(&self) -> Receiver<Self::Input>;

    /// Gets the [`Receiver`] for the existing orders.
    fn orders_receiver(&self) -> Receiver<Vec<ManagedOrder>>;

    /// Gets the [`Receiver`] for the existing orders.
    fn shutdown_receiver(&self) -> Receiver<bool>;

    /// Gets the [`Sender`] for the actions.
    fn action_sender(&self) -> Arc<Sender<Action>>;

    /// Gets the order layer count.
    ///
    /// This value represents the number of orders to be submitted on each side of the book.
    fn order_layer_count(&self) -> usize;

    /// Gets the layer distance in basis points.
    ///
    /// This value will be used to calculate the price of each order layer, starting from the configured spread.
    fn layer_spacing_bps(&self) -> u16;

    /// Gets the time in force value in seconds.
    ///
    /// This value will be used to judge if certain orders are expired.
    fn time_in_force(&self) -> u64;

    /// Gets the managed orders reader.
    async fn context_reader(&self) -> RwLockReadGuard<Self::Input>;

    /// Gets the managed orders writer.
    async fn context_writer(&self) -> RwLockWriteGuard<Self::Input>;

    /// Gets the managed orders reader.
    async fn managed_orders_reader(&self) -> RwLockReadGuard<Vec<ManagedOrder>>;

    /// Gets the managed orders writer.
    async fn managed_orders_writer(&self) -> RwLockWriteGuard<Vec<ManagedOrder>>;

    /// Starts the [`OrderManager`],
    async fn start(&self) -> Result<(), MakerError> {
        let mut context_receiver = self.context_receiver();
        let mut orders_receiver = self.orders_receiver();
        let mut shutdown_receiver = self.shutdown_receiver();
        let symbol = self.symbol();

        info!("{} - [{}] Starting maker..", type_name::<Self>(), symbol);

        loop {
            tokio::select! {
                ctx_update = context_receiver.recv() => {
                    match ctx_update {
                        Ok(ctx) => {
                            let mut context_writer = self.context_writer().await;
                            *context_writer = ctx;
                            drop(context_writer);
                            match self.pulse().await {
                                Ok(res) => {
                                    info!("{} - [{}] Maker pulse: {:?}", type_name::<Self>(), symbol, res);
                                },
                                Err(e) => {
                                    warn!("{} - [{}] There was an error processing maker pulse: {:?}", type_name::<Self>(), symbol, e.to_string());
                                }
                            };
                            tokio::time::sleep(Duration::from_millis(2500)).await;
                        },
                        Err(e) => {
                            warn!("{} - [{}] There was an error receiving maker input context update.", type_name::<Self>(), symbol);
                        }
                    }
                }
                orders_update = orders_receiver.recv() => {
                    match orders_update {
                        Ok(orders) => {
                            let mut orders_writer = self.managed_orders_writer().await;
                            *orders_writer = orders;
                            drop(orders_writer);
                            match self.pulse().await {
                                Ok(res) => {
                                    info!("{} - [{}] Maker pulse: {:?}", type_name::<Self>(), symbol, res);
                                },
                                Err(e) => {
                                    warn!("{} - [{}] There was an error processing maker pulse: {:?}", type_name::<Self>(), symbol, e.to_string());
                                }
                            };
                            tokio::time::sleep(Duration::from_millis(2500)).await;
                        },
                        Err(e) => {
                            warn!("{} - [{}] There was an error receiving order manager orders update.", type_name::<Self>(), symbol);
                        }
                    }
                }
                _ = shutdown_receiver.recv() => {
                    info!("{} - [{}] Shutdown signal received, stopping..", type_name::<Self>(), symbol);
                    break;
                }
            }
        }

        Ok(())
    }

    /// Gets expired orders which should be cancelled. This is according to time in force timestmap.
    fn get_expired_orders(&self, orders: &[ManagedOrder]) -> Vec<CandidateCancel> {
        let mut expired_orders = Vec::new();
        let cur_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        for order in orders.iter() {
            if order.max_ts < cur_ts {
                info!(
                    "{} - [{}] Candidate cancel - {:?} - Order Id: {} - Client Id: {}",
                    type_name::<Self>(),
                    self.symbol(),
                    order.side,
                    order.order_id,
                    order.client_order_id
                );
                expired_orders.push(CandidateCancel {
                    order_id: order.order_id,
                    client_order_id: order.client_order_id,
                    side: order.side,
                    layer: order.layer,
                });
            }
        }

        expired_orders
    }

    /// Gets stale orders which should be cancelled.
    ///
    /// This is done in a very lazy way, simply by comparing price and size of existing orders at the same layer.
    fn get_stale_orders(
        &self,
        orders: &[ManagedOrder],
        candidate_placements: &[CandidatePlacement],
    ) -> Vec<CandidateCancel> {
        let mut stale_orders = Vec::new();

        for order in orders.iter() {
            match candidate_placements
                .iter()
                .find(|p| order.layer == p.layer && order.side == p.side)
            {
                Some(equivalent_candidate) => {
                    // we will simply check the size and price of the order
                    if equivalent_candidate.price != order.price
                        || equivalent_candidate.base_quantity != order.base_quantity
                    {
                        info!(
                            "{} - [{}] Candidate cancel - {:?} - Layer: {} - Order Id: {} - Client Id: {}",
                            type_name::<Self>(),
                            self.symbol(),
                            order.side,
                            order.layer,
                            order.order_id,
                            order.client_order_id
                        );
                        stale_orders.push(CandidateCancel {
                            order_id: order.order_id,
                            client_order_id: order.client_order_id,
                            side: order.side,
                            layer: order.layer,
                        })
                    }
                }
                None => {
                    // if this order does not have an equivalent candidate we are going to assume that this order should not be here
                    info!(
                        "{} - [{}] Candidate cancel - {:?} - Layer: {} - Order Id: {} - Client Id: {}",
                        type_name::<Self>(),
                        self.symbol(),
                        order.side,
                        order.layer,
                        order.order_id,
                        order.client_order_id
                    );
                    stale_orders.push(CandidateCancel {
                        order_id: order.order_id,
                        client_order_id: order.client_order_id,
                        side: order.side,
                        layer: order.layer,
                    });
                    ()
                }
            };
        }

        stale_orders
    }

    /// Calculates the order size.
    fn get_order_size(
        &self,
        layer_count: usize,
        layer: usize,
        total_quote_size: I80F48,
        prev_order_size: I80F48,
    ) -> I80F48 {
        let layer_count = I80F48::from_num(layer_count);
        let layer = I80F48::from_num(layer);
        total_quote_size
            .div(layer_count.sub(layer).add(I80F48::ONE).mul(layer_count))
            .add(prev_order_size)
    }

    /// Gets new orders which should be placed.
    fn get_new_orders(
        &self,
        quote_volumes: &QuoteVolumes,
        spread_info: &SpreadInfo,
    ) -> Vec<CandidatePlacement> {
        let new_asks = self.get_new_asks(quote_volumes, spread_info);
        let new_bids = self.get_new_bids(quote_volumes, spread_info);

        [new_bids, new_asks].concat()
    }

    /// Gets the new bids to place.
    fn get_new_bids(
        &self,
        quote_volumes: &QuoteVolumes,
        spread_info: &SpreadInfo,
    ) -> Vec<CandidatePlacement> {
        let num_layers = self.order_layer_count();
        let layer_spacing = self.layer_spacing_bps();
        let layer_bps = I80F48::from(BPS_UNIT)
            .add(I80F48::from(layer_spacing))
            .div(I80F48::from(BPS_UNIT));

        let mut new_orders = Vec::new();
        let mut prev_order_size = I80F48::ZERO;
        let mut prev_order_price = I80F48::ZERO;

        for i in 1..num_layers + 1 {
            let order_size =
                self.get_order_size(num_layers, i, quote_volumes.bid_size, prev_order_size);
            let order_price = if i == 1 {
                spread_info.bid
            } else {
                prev_order_price.div(layer_bps)
            };

            info!(
                "{} - [{}] Candidate placement - BID {:.5} @ {:.5}",
                type_name::<Self>(),
                self.symbol(),
                order_size,
                order_price,
            );

            new_orders.push(CandidatePlacement {
                side: Side::Bid,
                price: order_price,
                base_quantity: order_size,
                max_quote_quantity: order_price.checked_mul(order_size).unwrap(),
                layer: i,
                ..Default::default()
            });
            prev_order_price = order_price;
            prev_order_size = order_size;
        }

        new_orders
    }

    /// Gets the new asks to place.
    fn get_new_asks(
        &self,
        quote_volumes: &QuoteVolumes,
        spread_info: &SpreadInfo,
    ) -> Vec<CandidatePlacement> {
        let num_layers = self.order_layer_count();
        let layer_spacing = self.layer_spacing_bps();
        let layer_bps = I80F48::from(BPS_UNIT)
            .add(I80F48::from(layer_spacing))
            .div(I80F48::from(BPS_UNIT));

        let mut new_orders = Vec::new();
        let mut prev_order_size = I80F48::ZERO;
        let mut prev_order_price = I80F48::ZERO;

        for i in 1..num_layers + 1 {
            let order_size =
                self.get_order_size(num_layers, i, quote_volumes.bid_size, prev_order_size);
            let order_price = if i == 1 {
                spread_info.ask
            } else {
                prev_order_price.mul(layer_bps)
            };

            info!(
                "{} - [{}] Candidate placement - ASK {:.5} @ {:.5}",
                type_name::<Self>(),
                self.symbol(),
                order_size,
                order_price,
            );

            new_orders.push(CandidatePlacement {
                side: Side::Ask,
                price: order_price,
                base_quantity: order_size,
                max_quote_quantity: order_price.checked_mul(order_size).unwrap(),
                layer: i,
                ..Default::default()
            });
            prev_order_price = order_price;
            prev_order_size = order_size;
        }

        new_orders
    }

    /// Cancels expired orders according to their time in force.
    async fn cancel_expired_orders(&self, orders: &[ManagedOrder]) -> Result<usize, MakerError> {
        let action_sender = self.action_sender();
        let expired_orders = self.get_expired_orders(&orders);

        info!(
            "{} - [{}] Updating orders..",
            type_name::<Self>(),
            self.symbol(),
        );

        let num_expired_orders = if !expired_orders.is_empty() {
            info!(
                "{} - [{}] Cancelling {} expired orders.",
                type_name::<Self>(),
                self.symbol(),
                expired_orders.len()
            );

            match action_sender.send(Action::CancelOrders(expired_orders.clone())) {
                Ok(_) => {
                    info!(
                        "{} - [{}] Sucessfully sent action to order manager..",
                        type_name::<Self>(),
                        self.symbol(),
                    );
                }
                Err(e) => {
                    return Err(MakerError::ActionSendError(e));
                }
            };
            expired_orders.len()
        } else {
            info!(
                "{} - [{}] There are no expired orders.",
                type_name::<Self>(),
                self.symbol(),
            );
            0
        };

        Ok(num_expired_orders)
    }

    /// Places new orders.
    async fn place_new_orders(
        &self,
        quote_volumes: &QuoteVolumes,
        spread_info: &SpreadInfo,
    ) -> Result<usize, MakerError> {
        let action_sender = self.action_sender();
        let candidate_placements = self.get_new_orders(quote_volumes, spread_info);

        info!(
            "{} - [{}] Submitting {} new orders.",
            type_name::<Self>(),
            self.symbol(),
            candidate_placements.len()
        );
        match action_sender.send(Action::PlaceOrders(candidate_placements.clone())) {
            Ok(_) => {
                info!(
                    "{} - [{}] Sucessfully sent action to order manager..",
                    type_name::<Self>(),
                    self.symbol(),
                );
            }
            Err(e) => {
                return Err(MakerError::ActionSendError(e));
            }
        };

        Ok(candidate_placements.len())
    }

    /// Filters candidate placements.
    fn filter_candidate_placements(
        &self,
        candidate_placements: &[CandidatePlacement],
        candidate_cancels: &[CandidateCancel],
    ) -> Result<Vec<CandidatePlacement>, MakerError> {
        let mut final_candidates = Vec::new();

        // iterate over the candidate placements and see if there is a candidate cancel for the same layer
        // if there is one, we will actually want to submit a new order for that layer
        for candidate_placement in candidate_placements.iter() {
            match candidate_cancels.iter().find(|candidate_cancel| {
                candidate_cancel.layer == candidate_placement.layer
                    && candidate_cancel.side == candidate_placement.side
            }) {
                Some(c) => {
                    info!(
                        "{} - [{}] Final candidate - {:?} - {:.5} @ {:.5}",
                        type_name::<Self>(),
                        self.symbol(),
                        candidate_placement.side,
                        candidate_placement.base_quantity,
                        candidate_placement.price,
                    );
                    final_candidates.push(candidate_placement.clone());
                }
                None => continue,
            }
        }

        Ok(final_candidates)
    }

    /// Updates the maker orders.
    async fn update_orders(
        &self,
        quote_volumes: &QuoteVolumes,
        spread_info: &SpreadInfo,
        orders: &[ManagedOrder],
    ) -> Result<MakerPulseResult, MakerError> {
        let action_sender = self.action_sender();

        /// if we have no orders we need to submit new ones
        if orders.is_empty() {
            match self.place_new_orders(quote_volumes, spread_info).await {
                Ok(num_new_orders) => {
                    return Ok(MakerPulseResult {
                        num_new_orders,
                        num_cancelled_orders: 0,
                    });
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }

        /// otherwise we have a few things to do
        /// 1. first we will see if there are expired orders
        let num_expired_canceled = match self.cancel_expired_orders(orders).await {
            Ok(num_canceled) => num_canceled,
            Err(e) => {
                return Err(e);
            }
        };

        /// 2. secondly, we get what would be our newest desired orders
        let candidate_placements = self.get_new_orders(quote_volumes, spread_info);

        /// then we see if any of the existing orders are stale
        let stale_orders = self.get_stale_orders(&orders, &candidate_placements);
        let num_cancelled_stale_orders = if !stale_orders.is_empty() {
            info!(
                "{} - [{}] Cancelling {} stale orders.",
                type_name::<Self>(),
                self.symbol(),
                stale_orders.len()
            );
            match action_sender.send(Action::CancelOrders(stale_orders.clone())) {
                Ok(_) => {
                    info!(
                        "{} - [{}] Sucessfully sent action to order manager..",
                        type_name::<Self>(),
                        self.symbol(),
                    );
                }
                Err(e) => {
                    return Err(MakerError::ActionSendError(e));
                }
            };
            stale_orders.len()
        } else {
            0
        };

        let final_candidates =
            match self.filter_candidate_placements(&candidate_placements, &stale_orders) {
                Ok(a) => a,
                Err(e) => {
                    return Err(e);
                }
            };

        let num_new_orders = if !final_candidates.is_empty() {
            info!(
                "{} - [{}] Submitting {} final candidates.",
                type_name::<Self>(),
                self.symbol(),
                final_candidates.len()
            );
            match action_sender.send(Action::PlaceOrders(final_candidates.clone())) {
                Ok(_) => {
                    info!(
                        "{} - [{}] Sucessfully sent action to order manager..",
                        type_name::<Self>(),
                        self.symbol(),
                    );
                }
                Err(e) => {
                    return Err(MakerError::ActionSendError(e));
                }
            };
            final_candidates.len()
        } else {
            0
        };

        /// only after checking for stale orders do we see which we want to submit
        Ok(MakerPulseResult {
            num_cancelled_orders: num_expired_canceled + num_cancelled_stale_orders,
            num_new_orders,
        })
    }

    /// Triggers a pulse, prompting the maker to perform it's work cycle.
    async fn pulse(&self) -> Result<MakerPulseResult, MakerError>;

    /// Gets the symbol this [`Maker`] represents.
    fn symbol(&self) -> &str;
}
