use async_trait::async_trait;
use cypher_client::Side;
use fixed::types::I80F48;
use log::{debug, info, warn};
use solana_client::{client_error::ClientError, nonblocking::rpc_client::RpcClient};
use solana_sdk::{instruction::Instruction, signature::Keypair};
use std::{
    ops::{Add, Div, Mul, Sub},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::sync::broadcast::Receiver;

use crate::{
    common::orders::InflightCancel,
    market_maker::constants::BPS_UNIT,
    utils::transactions::{send_cancels, send_placements},
};

use super::{
    context::OrdersContext,
    inventory::{InventoryManager, QuoteVolumes, SpreadInfo},
    order_manager::OrderManager,
    orders::{CandidateCancel, CandidatePlacement, ManagedOrder, OrdersInfo},
    Identifier,
};

/// Represents the result of a [`Maker`]'s pulse.
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
    Client(#[from] ClientError),
}

/// Defines shared functionality that different makers should implement.
#[async_trait]
pub trait Maker: Send + Sync + OrderManager + Identifier {
    /// The input type for the [`Maker`].
    type Input: Clone + Send + Sync + OrdersContext;

    /// The input type for the [`InventoryManager`].
    type InventoryManagerInput: Clone + Send + Sync;

    /// Gets the [`RpcClient`].
    ///
    /// OBS: If there is a desire to turn this entire thing somewhat agnostic, might be worthwhile
    /// switching this for an `ExchangeAdapter` trait which abstracts away the transaction building and submission.
    fn rpc_client(&self) -> Arc<RpcClient>;

    /// Gets the [`Keypair`].
    ///
    /// OBS: If there is a desire to turn this entire thing somewhat agnostic, might be worthwhile
    /// switching this for an `ExchangeAdapter` trait which abstracts away the transaction building and submission.
    fn signer(&self) -> Arc<Keypair>;

    /// Gets the inventory manager for the [`Maker`].
    fn inventory_manager(&self) -> Arc<dyn InventoryManager<Input = Self::InventoryManagerInput>>;

    /// Gets the [`Receiver`] for the respective [`Self::Input`] data type.
    fn context_receiver(&self) -> Receiver<<Self as Maker>::Input>;

    /// Gets the [`Receiver`] for the existing orders.
    fn shutdown_receiver(&self) -> Receiver<bool>;

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

    /// Starts the [`OrderManager`],
    async fn start(&self) -> Result<(), MakerError> {
        let mut context_receiver = self.context_receiver();
        let mut shutdown_receiver = self.shutdown_receiver();
        let symbol = self.symbol();

        info!("[{}] Starting maker..", symbol);

        loop {
            tokio::select! {
                ctx_update = context_receiver.recv() => {
                    match ctx_update {
                        Ok(ctx) => {
                            self.process_update(&ctx).await;

                            match self.pulse(&ctx).await {
                                Ok(res) => {
                                    info!("[{}] Maker pulse: {:?}",  symbol, res);
                                },
                                Err(e) => {
                                    warn!("[{}] There was an error processing maker pulse. Error: {:?}", symbol, e);
                                }
                            };
                        },
                        Err(e) => {
                            warn!("[{}] There was an error receiving maker input context update. Error: {:?}", symbol, e);
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

    /// Gets expired orders which should be cancelled. This is according to time in force timestmap.
    fn get_expired_orders(&self, orders: &[ManagedOrder]) -> Vec<CandidateCancel> {
        let mut expired_orders = Vec::new();
        let cur_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        for order in orders.iter() {
            if order.max_ts < cur_ts || order.layer == usize::MAX {
                debug!(
                    "[{}] Candidate cancel - {:?} - Order Id: {} - Client Id: {}",
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
                        debug!(
                            "[{}] Candidate cancel - {:?} - Layer: {} - Order Id: {} - Client Id: {}",
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
                    debug!(
                        "[{}] Candidate cancel - {:?} - Layer: {} - Order Id: {} - Client Id: {}",
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
            debug!(
                "[{}] Candidate placement - BID {:.5} @ {:.5}",
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
                self.get_order_size(num_layers, i, quote_volumes.ask_size, prev_order_size);
            let order_price = if i == 1 {
                spread_info.ask
            } else {
                prev_order_price.mul(layer_bps)
            };

            debug!(
                "[{}] Candidate placement - ASK {:.5} @ {:.5}",
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
        let expired_orders = self.get_expired_orders(orders);

        info!("[{}] Updating orders..", self.symbol(),);

        let num_expired_orders = if !expired_orders.is_empty() {
            info!(
                "[{}] Cancelling {} expired orders.",
                self.symbol(),
                expired_orders.len()
            );

            match self.cancel_orders(&expired_orders).await {
                Ok(_) => {
                    info!("[{}] Sucessfully cancelled orders..", self.symbol(),);
                }
                Err(e) => {
                    return Err(e);
                }
            };
            expired_orders.len()
        } else {
            info!("[{}] There are no expired orders.", self.symbol(),);
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
        let candidate_placements = self.get_new_orders(quote_volumes, spread_info);

        info!(
            "[{}] Submitting {} new orders.",
            self.symbol(),
            candidate_placements.len()
        );
        match self.place_orders(&candidate_placements).await {
            Ok(_) => {
                info!("[{}] Sucessfully placed new orders..", self.symbol(),);
            }
            Err(e) => {
                return Err(e);
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
                Some(_c) => {
                    info!(
                        "[{}] Final candidate - {:?} - {:.5} @ {:.5}",
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
        orders: &OrdersInfo,
    ) -> Result<MakerPulseResult, MakerError> {
        if orders.open_orders.is_empty()
            && (!orders.inflight_orders.is_empty() || !orders.inflight_cancels.is_empty())
        {
            return Ok(MakerPulseResult {
                num_new_orders: 0,
                num_cancelled_orders: 0,
            });
        }

        // if we have no orders we need to submit new ones
        if orders.open_orders.is_empty() {
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

        // otherwise we have a few things to do
        // 1. first we will see if there are expired orders
        let num_expired_canceled = match self.cancel_expired_orders(&orders.open_orders).await {
            Ok(num_canceled) => num_canceled,
            Err(e) => {
                return Err(e);
            }
        };

        // 2. secondly, we get what would be our newest desired orders
        let candidate_placements = self.get_new_orders(quote_volumes, spread_info);

        // then we see if any of the existing orders are stale
        let stale_orders = self.get_stale_orders(&orders.open_orders, &candidate_placements);
        let num_cancelled_stale_orders = if !stale_orders.is_empty() {
            info!(
                "[{}] Cancelling {} stale orders.",
                self.symbol(),
                stale_orders.len()
            );
            match self.cancel_orders(&stale_orders).await {
                Ok(_) => {
                    info!("[{}] Sucessfully cancelled orders..", self.symbol(),);
                }
                Err(e) => {
                    return Err(e);
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
                "[{}] Submitting {} final candidates.",
                self.symbol(),
                final_candidates.len()
            );
            match self.place_orders(&final_candidates).await {
                Ok(_) => {
                    info!("[{}] Sucessfully placed orders..", self.symbol(),);
                }
                Err(e) => {
                    return Err(e);
                }
            };
            final_candidates.len()
        } else {
            0
        };

        // only after checking for stale orders do we see which we want to submit
        Ok(MakerPulseResult {
            num_cancelled_orders: num_expired_canceled + num_cancelled_stale_orders,
            num_new_orders,
        })
    }

    async fn check_inflight_orders(
        &self,
        order_placements: &[CandidatePlacement],
    ) -> Vec<CandidatePlacement> {
        let inflight_orders_reader = self.inflight_orders_reader().await;
        let mut filtered_candidates = Vec::new();

        for op in order_placements.iter() {
            match inflight_orders_reader.iter().find(|inflight_order| {
                op.layer == inflight_order.layer && op.side == inflight_order.side
            }) {
                Some(_) => (),
                None => {
                    filtered_candidates.push(op.clone());
                }
            }
        }

        info!(
            "[{}] Filtered {} duplicate order placements.",
            self.symbol(),
            order_placements.len() - filtered_candidates.len()
        );

        filtered_candidates
    }

    /// Submits new orders, adding them to the inflight orders tracker.
    async fn place_orders(
        &self,
        order_placements: &[CandidatePlacement],
    ) -> Result<(), MakerError> {
        let filtered_placements = self.check_inflight_orders(order_placements).await;

        if !filtered_placements.is_empty() {
            let signer = self.signer();
            let rpc_client = self.rpc_client();
            let (ixs, managed_orders) = self.build_new_order_ixs(&filtered_placements).await;
            match send_placements(
                &rpc_client,
                filtered_placements.clone(),
                ixs,
                &signer,
                false,
            )
            .await
            {
                Ok(sigs) => {
                    // add these submitted orders to the inflight orders tracker
                    let mut inflight_orders = self.inflight_orders_writer().await;
                    for sig in sigs.iter() {
                        if let Some(signature) = sig.signature {
                            info!(
                                "[{}] Sucessfully submitted transaction. Signature: {}.",
                                self.symbol(),
                                signature
                            );
                            let mut candidates_submitted = Vec::new();
                            for candidate in sig.candidates.iter() {
                                match managed_orders.iter().find(|o| {
                                    o.layer == candidate.layer && o.side == candidate.side
                                }) {
                                    Some(order) => {
                                        info!(
                                            "[{}] Sucessfully submitted order: {:?}.",
                                            self.symbol(),
                                            candidate
                                        );
                                        candidates_submitted.push(order.clone());
                                    }
                                    None => {
                                        continue;
                                    }
                                }
                            }
                            inflight_orders.extend(candidates_submitted);
                        } else {
                            for order in sig.candidates.iter() {
                                warn!("[{}] Failed to submit order: {:?}", self.symbol(), order);
                            }
                        }
                    }
                    drop(inflight_orders);
                    Ok(())
                }
                Err(e) => Err(MakerError::Client(e)),
            }
        } else {
            Ok(())
        }
    }

    /// Builds cancel order instructions from the given [`CandidatePlacement`]s.
    async fn build_new_order_ixs(
        &self,
        order_placements: &[CandidatePlacement],
    ) -> (Vec<Instruction>, Vec<ManagedOrder>);

    async fn check_inflight_cancels(
        &self,
        order_cancels: &[CandidateCancel],
    ) -> Vec<CandidateCancel> {
        let inflight_cancels = self.inflight_cancels_reader().await;
        let mut filtered_candidates = Vec::new();

        for oc in order_cancels.iter() {
            match inflight_cancels.iter().find(|inflight_cancel| {
                oc.side == inflight_cancel.side
                    && (oc.client_order_id == inflight_cancel.client_order_id
                        || oc.order_id == inflight_cancel.order_id)
            }) {
                Some(_) => (),
                None => {
                    filtered_candidates.push(oc.clone());
                }
            }
        }

        info!(
            "[{}] Filtered {} duplicate order cancels.",
            self.symbol(),
            order_cancels.len() - filtered_candidates.len()
        );

        filtered_candidates
    }

    /// Submits new orders, adding them to the inflight orders tracker.
    async fn cancel_orders(&self, order_cancels: &[CandidateCancel]) -> Result<(), MakerError> {
        let filtered_cancels = self.check_inflight_cancels(order_cancels).await;

        if !filtered_cancels.is_empty() {
            let signer = self.signer();
            let rpc_client = self.rpc_client();
            let ixs = self.build_cancel_order_ixs(&filtered_cancels).await;
            match send_cancels(&rpc_client, filtered_cancels.clone(), ixs, &signer, false).await {
                Ok(sigs) => {
                    // add these cancels to the inflight cancels tracker
                    let mut inflight_cancels = self.inflight_cancels_writer().await;
                    let cur_ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
                    for sig in sigs.iter() {
                        if let Some(signature) = sig.signature {
                            info!(
                                "[{}] Sucessfully submitted transaction. Signature: {}.",
                                self.symbol(),
                                signature
                            );
                            let mut candidates_submitted = Vec::new();
                            for candidate in sig.candidates.iter() {
                                match filtered_cancels.iter().find(|o| {
                                    o.layer == candidate.layer && o.side == candidate.side
                                }) {
                                    Some(_) => {
                                        info!(
                                            "[{}] Sucessfully submitted cancel: {:?}.",
                                            self.symbol(),
                                            candidate
                                        );
                                        candidates_submitted.push(InflightCancel {
                                            side: candidate.side,
                                            order_id: candidate.order_id,
                                            client_order_id: candidate.client_order_id,
                                            submitted_at: cur_ts.as_secs(),
                                        });
                                    }
                                    None => {
                                        continue;
                                    }
                                }
                            }
                            inflight_cancels.extend(candidates_submitted);
                        } else {
                            for order in sig.candidates.iter() {
                                warn!("[{}] Failed to submit cancel: {:?}", self.symbol(), order);
                            }
                        }
                    }
                    drop(inflight_cancels);
                    Ok(())
                }
                Err(e) => Err(MakerError::Client(e)),
            }
        } else {
            Ok(())
        }
    }

    /// Builds cancel order instructions from the given [`CandidateCancel`]s.
    async fn build_cancel_order_ixs(&self, order_cancels: &[CandidateCancel]) -> Vec<Instruction>;

    /// Gets confirmed and inflight orders.
    async fn get_orders(&self) -> OrdersInfo {
        let inflight_cancels = self.inflight_cancels_reader().await;
        let managed_orders = self.managed_orders_reader().await;
        let inflight_orders = self.inflight_orders_reader().await;
        let open_orders = self.open_orders_reader().await;

        let mut orders = Vec::new();

        for order in managed_orders.iter() {
            match inflight_cancels.iter().find(|c| {
                c.order_id == order.order_id || c.client_order_id == order.client_order_id
            }) {
                Some(_) => (),
                None => orders.push(order.clone()),
            }
        }

        for order in open_orders.iter() {
            match orders.iter().find(|o| o.order_id == order.order_id) {
                Some(_) => (),
                None => orders.push(ManagedOrder {
                    price_lots: order.price,
                    base_quantity_lots: order.base_quantity,
                    quote_quantity_lots: order.quote_quantity,
                    price: I80F48::ZERO,
                    base_quantity: I80F48::ZERO,
                    max_quote_quantity: I80F48::ZERO,
                    max_ts: order.max_ts,
                    submitted_at: u64::MIN,
                    client_order_id: order.client_order_id,
                    order_id: order.order_id,
                    side: order.side,
                    ..Default::default()
                }),
            }
        }

        OrdersInfo {
            open_orders: orders,
            inflight_orders: inflight_orders.to_vec(),
            inflight_cancels: inflight_cancels.to_vec(),
        }
    }

    /// Triggers a pulse, prompting the [`Maker`] to perform it's work cycle.
    /// This is also where any logic that needs to be performed before the generic [`Maker`] work should be performed.
    async fn pulse(&self, input: &<Self as Maker>::Input) -> Result<MakerPulseResult, MakerError>;
}
