use async_trait::async_trait;
use cypher_client::Side;
use cypher_utils::{contexts::Order, utils::send_transactions};
use fixed::types::I80F48;
use log::{info, warn};
use solana_client::{client_error::ClientError, nonblocking::rpc_client::RpcClient};
use solana_sdk::{
    instruction::Instruction,
    signature::{Keypair, Signature},
};
use std::{any::type_name, sync::Arc};
use thiserror::Error;
use tokio::sync::{
    broadcast::{error::SendError, Receiver, Sender},
    RwLockReadGuard, RwLockWriteGuard,
};

use super::context::{builder::ContextBuilder, OrdersContext};

#[derive(Default, Debug, Clone)]
pub struct CandidateCancel {
    /// The order's id.
    pub order_id: u128,
    /// The order's client id.
    pub client_order_id: u64,
    /// The order's side.
    pub side: Side,
    /// The layer that this order represents.
    pub layer: usize,
}

#[derive(Default, Debug, Clone)]
pub struct CandidatePlacement {
    /// The order's price.
    pub price: I80F48,
    /// The order's base quantity.
    pub base_quantity: I80F48,
    /// The order's max quote quantity.
    pub max_quote_quantity: I80F48,
    /// The order's time in force timestamp.
    pub max_ts: u64,
    /// The order's client id.
    pub client_order_id: u64,
    /// The order's side.
    pub side: Side,
    /// The layer that this order represents.
    pub layer: usize,
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct InflightCancel {
    /// The order id.
    pub order_id: u128,
    /// The client order id.
    pub client_order_id: u64,
    /// The order side.
    pub side: Side,
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct ManagedOrder {
    /// The order's price.
    pub price_lots: u64,
    /// The order's max base quantity.
    pub base_quantity_lots: u64,
    /// The order's max quote quantity.
    pub quote_quantity_lots: u64,
    /// The order's price.
    pub price: I80F48,
    /// The order's base quantity.
    pub base_quantity: I80F48,
    /// The order's max quote quantity.
    pub max_quote_quantity: I80F48,
    /// The order's time in force timestamp.
    pub max_ts: u64,
    /// The order's client id.
    pub client_order_id: u64,
    /// The order's id.
    pub order_id: u128,
    /// The side of the order.
    pub side: Side,
    /// The layer of the order.
    pub layer: usize,
}

#[derive(Debug, Error)]
pub enum OrderManagerError {
    #[error(transparent)]
    ClientError(#[from] ClientError),
    #[error("Send error: {:?}", self)]
    OrdersSendError(SendError<Vec<ManagedOrder>>),
}

/// Actions that the [`OrderManager`] can perform.
#[derive(Debug, Clone)]
pub enum Action {
    /// An action to cancel orders.
    CancelOrders(Vec<CandidateCancel>),
    /// An action to place orders.
    PlaceOrders(Vec<CandidatePlacement>),
}

/// Defines shared functionality that different order managers should implement.
#[async_trait]
pub trait OrderManager: Send + Sync {
    /// The input type of the [`OrderManager`] data necessary to maintain an up-to-date state.
    type Input: Clone + Send + Sync + OrdersContext;

    /// Gets the time in force value in seconds.
    ///
    /// This value will be used to judge if certain orders are expired.
    fn time_in_force(&self) -> u64;

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

    /// Gets the [`Receiver`] for the respective [`Self::Input`] data type.
    fn context_receiver(&self) -> Receiver<Self::Input>;

    /// Gets the [`Receiver`] for the [`Action`]s.
    fn action_sender(&self) -> Arc<Sender<Action>>;

    /// Gets the [`Receiver`] for the [`Action`]s.
    fn action_receiver(&self) -> Receiver<Action>;

    /// Gets the shutdown [`Receiver`].
    fn shutdown_receiver(&self) -> Receiver<bool>;

    /// Gets the [`Sender`] for the existing orders.
    fn sender(&self) -> Arc<Sender<Vec<ManagedOrder>>>;

    /// Gets the managed orders reader.
    async fn managed_orders_reader(&self) -> RwLockReadGuard<Vec<ManagedOrder>>;

    /// Gets the managed orders writer.
    async fn managed_orders_writer(&self) -> RwLockWriteGuard<Vec<ManagedOrder>>;

    /// Gets the open orders reader.
    async fn open_orders_reader(&self) -> RwLockReadGuard<Vec<Order>>;

    /// Gets the open orders writer.
    async fn open_orders_writer(&self) -> RwLockWriteGuard<Vec<Order>>;

    /// Gets the inflight order placements reader.
    async fn inflight_orders_reader(&self) -> RwLockReadGuard<Vec<ManagedOrder>>;

    /// Gets the inflight order placements writer.
    async fn inflight_orders_writer(&self) -> RwLockWriteGuard<Vec<ManagedOrder>>;

    /// Gets the inflight order cancels reader.
    async fn inflight_cancels_reader(&self) -> RwLockReadGuard<Vec<InflightCancel>>;

    /// Gets the inflight order cancels writer.
    async fn inflight_cancels_writer(&self) -> RwLockWriteGuard<Vec<InflightCancel>>;

    /// Starts the [`OrderManager`],
    async fn start(&self) -> Result<(), OrderManagerError> {
        let mut context_receiver = self.context_receiver();
        let mut action_receiver = self.action_receiver();
        let mut shutdown_receiver = self.shutdown_receiver();
        let symbol = self.symbol();

        info!(
            "{} - [{}] Starting order manager..",
            type_name::<Self>(),
            symbol
        );

        loop {
            tokio::select! {
                ctx_update = context_receiver.recv() => {
                    match ctx_update {
                        Ok(ctx) => {
                            match self.process_update(&ctx).await {
                                Ok(()) => {
                                    match self.send_update().await {
                                        Ok(()) => {
                                            info!("{} - [{}] Successfully sent orders update.", type_name::<Self>(), symbol, );
                                        }
                                        Err(e) => {
                                            warn!("{} - [{}] There was an error sending orders update: {:?}", type_name::<Self>(), symbol, e.to_string());
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!("{} - [{}] There was an error processing order manager update: {:?}", type_name::<Self>(), symbol, e.to_string());
                                }
                            };
                        },
                        Err(e) => {
                            warn!("{} - [{}] There was an error receiving order manager input context update.", type_name::<Self>(), symbol);
                        }
                    }
                }
                action_update = action_receiver.recv() => {{
                    match action_update {
                        Ok(action) => {
                            match action {
                                Action::PlaceOrders(orders) => {
                                    match self.place_orders(&orders).await {
                                        Ok(()) => (),
                                        Err(e) => {
                                            warn!("{} - [{}] There was an error placing orders: {:?}", type_name::<Self>(), symbol, e);
                                        }
                                    }
                                },
                                Action::CancelOrders(cancels) => {
                                    match self.cancel_orders(&cancels).await {
                                        Ok(()) => (),
                                        Err(e) => {
                                            warn!("{} - [{}] There was an error canceling orders: {:?}", type_name::<Self>(), symbol, e);
                                        }
                                    }
                                }
                            }
                        },
                        Err(e) => {
                            warn!("{} - [{}] There was an error receiving action update.", type_name::<Self>(), symbol);
                        }
                    }
                }}
                _ = shutdown_receiver.recv() => {
                    info!("{} - [{}] Shutdown signal received, stopping..", type_name::<Self>(), symbol);
                    break;
                }
            }
        }

        Ok(())
    }

    async fn send_update(&self) -> Result<(), OrderManagerError> {
        let update_sender = self.sender();
        let orders = self.get_orders().await;
        match update_sender.send(orders.clone()) {
            Ok(_) => Ok(()),
            Err(e) => Err(OrderManagerError::OrdersSendError(e)),
        }
    }

    async fn update_inflight_cancels(&self, ctx: &Self::Input) -> Result<(), OrderManagerError> {
        let symbol = self.symbol();
        let mut inflight_cancels = self.inflight_cancels_writer().await;
        let mut inflight_cancels_to_remove: Vec<InflightCancel> = Vec::new();
        let ctx_open_orders = ctx.get_open_orders();

        info!(
            "{} - [{}] There are {} inflight cancels and {} orders on the book.",
            type_name::<Self>(),
            symbol,
            inflight_cancels.len(),
            ctx_open_orders.len()
        );

        for inflight_cancel in inflight_cancels.iter_mut() {
            // check if we still find any orders that match our "inflight cancels"
            // if so, we'll ignore them, otherwise we can consider them removed
            match ctx_open_orders.iter().find(|p| {
                p.client_order_id == inflight_cancel.client_order_id
                    || p.order_id == inflight_cancel.order_id
            }) {
                Some(_) => {}
                None => {
                    if !inflight_cancels_to_remove.contains(&inflight_cancel) {
                        info!(
                            "{} - [{}] Inflight cancel confirmed. Side: {:?} - Order ID: {} - Client Order ID: {}",
                            type_name::<Self>(),
                            symbol,
                            inflight_cancel.side,
                            inflight_cancel.order_id,
                            inflight_cancel.client_order_id
                        );
                        inflight_cancels_to_remove.push(inflight_cancel.clone());
                    }
                }
            }
        }

        info!(
            "{} - [{}] Found {} inflight cancels that have been confirmed..",
            type_name::<Self>(),
            symbol,
            inflight_cancels_to_remove.len(),
        );

        if !inflight_cancels_to_remove.is_empty() {
            let mut managed_orders = self.managed_orders_writer().await;
            // remove these confirmed cancels from the ones we are tracking
            for order in inflight_cancels_to_remove.iter() {
                let order_idx = inflight_cancels
                    .iter()
                    .position(|p| {
                        p.client_order_id == order.client_order_id
                            || p.order_id == order.order_id && p.side == order.side
                    })
                    .unwrap();
                inflight_cancels.remove(order_idx);

                match managed_orders.iter().position(|p| {
                    p.client_order_id == order.client_order_id
                        || p.order_id == order.order_id && p.side == order.side
                }) {
                    Some(i) => {
                        managed_orders.remove(i);
                    }
                    None => continue,
                }
            }
        }

        Ok(())
    }

    async fn update_inflight_orders(&self, ctx: &Self::Input) -> Result<(), OrderManagerError> {
        let symbol = self.symbol();
        let mut inflight_orders = self.inflight_orders_writer().await;
        let mut inflight_orders_to_remove: Vec<ManagedOrder> = Vec::new();
        let ctx_open_orders = ctx.get_open_orders();

        for inflight_order in inflight_orders.iter() {
            // check if any of our "inflight orders" have now been confirmed
            // and if so, we should remove them from our tracking
            match ctx_open_orders
                .iter()
                .find(|p| p.client_order_id == inflight_order.client_order_id)
            {
                Some(o) => {
                    let mut order = inflight_order.clone();
                    order.order_id = o.order_id;
                    if !inflight_orders_to_remove.contains(&order) {
                        info!(
                            "{} - [{}] Inflight order confirmed. Side: {:?} - Price: {} - Size: {} - Layer: {} - Order ID: {} - Client Order ID: {}",
                            type_name::<Self>(),
                            symbol,
                            inflight_order.side,
                            inflight_order.price,
                            inflight_order.base_quantity,
                            inflight_order.layer,
                            inflight_order.order_id,
                            inflight_order.client_order_id,
                        );
                        inflight_orders_to_remove.push(order);
                    }
                }
                None => {}
            };
        }

        info!(
            "{} - [{}] Found {} inflight orders that have been confirmed..",
            type_name::<Self>(),
            symbol,
            inflight_orders_to_remove.len(),
        );

        if !inflight_orders_to_remove.is_empty() {
            let mut managed_orders = self.managed_orders_writer().await;

            // remove these confirmed orders from the ones we are tracking
            for order in inflight_orders_to_remove.iter() {
                let order_idx = inflight_orders
                    .iter()
                    .position(|p| p.client_order_id == order.client_order_id)
                    .unwrap();
                inflight_orders.remove(order_idx);
                managed_orders.push(order.clone());
            }
        }

        Ok(())
    }

    /// Processes an update of the order manager.
    async fn process_update(&self, ctx: &Self::Input) -> Result<(), OrderManagerError> {
        let symbol = self.symbol();

        // update our tracking of inflight cancels
        match self.update_inflight_cancels(ctx).await {
            Ok(()) => (),
            Err(e) => {
                warn!(
                    "{} - [{}] There was an error updating inflight order cancels: {:?}",
                    type_name::<Self>(),
                    symbol,
                    e.to_string()
                )
            }
        }
        // update our tracking of inflight orders
        match self.update_inflight_orders(ctx).await {
            Ok(()) => (),
            Err(e) => {
                warn!(
                    "{} - [{}] There was an error updating inflight order placements: {:?}",
                    type_name::<Self>(),
                    symbol,
                    e.to_string()
                )
            }
        }

        let mut managed_orders = self.managed_orders_writer().await;
        let ctx_open_orders = ctx.get_open_orders();

        // if this happens we will assume this is after our start-up
        // we'll take any existing on-chain order and add them to managed orders so they end up getting cancelled
        if managed_orders.is_empty() {
            info!(
                "{} - [{}] There are no managed orders, adding {} confirmed open orders..",
                type_name::<Self>(),
                symbol,
                ctx_open_orders.len()
            );
            for order in ctx_open_orders.iter() {
                managed_orders.push(ManagedOrder {
                    price_lots: order.price,
                    base_quantity_lots: order.base_quantity,
                    quote_quantity_lots: order.quote_quantity,
                    price: I80F48::ZERO,
                    base_quantity: I80F48::ZERO,
                    max_quote_quantity: I80F48::ZERO,
                    max_ts: u64::MIN,
                    client_order_id: order.client_order_id,
                    order_id: order.order_id,
                    side: order.side,
                    layer: usize::MIN,
                });
            }
        }

        let mut open_orders = self.open_orders_writer().await;
        *open_orders = ctx_open_orders.to_vec();

        info!(
            "{} - [{}] There are {} confirmed open orders..",
            type_name::<Self>(),
            symbol,
            ctx_open_orders.len()
        );

        Ok(())
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
            "{} - [{}] Filtered {} duplicate order placements.",
            type_name::<Self>(),
            self.symbol(),
            order_placements.len() - filtered_candidates.len()
        );

        filtered_candidates
    }

    /// Submits new orders, adding them to the inflight orders tracker.
    async fn place_orders(
        &self,
        order_placements: &[CandidatePlacement],
    ) -> Result<(), OrderManagerError> {
        let filtered_placements = self.check_inflight_orders(order_placements).await;

        if !filtered_placements.is_empty() {
            let signer = self.signer();
            let rpc_client = self.rpc_client();
            let (ixs, managed_orders) = self.build_new_order_ixs(&filtered_placements).await;
            match send_transactions(&rpc_client, ixs, &signer, false).await {
                Ok(sigs) => {
                    // add these submitted orders to the inflight orders tracker
                    let mut inflight_orders = self.inflight_orders_writer().await;
                    inflight_orders.extend(managed_orders);
                    drop(inflight_orders);
                    for sig in sigs.iter() {
                        info!(
                            "{} - [{}] Sucessfully submitted transaction. Signature: {}.",
                            type_name::<Self>(),
                            self.symbol(),
                            sig
                        );
                    }
                    match self.send_update().await {
                        Ok(()) => {
                            info!(
                                "{} - [{}] Successfully sent orders update.",
                                type_name::<Self>(),
                                self.symbol(),
                            );
                        }
                        Err(e) => {
                            warn!(
                                "{} - [{}] There was an error sending orders update: {:?}",
                                type_name::<Self>(),
                                self.symbol(),
                                e.to_string()
                            );
                        }
                    }
                    Ok(())
                }
                Err(e) => Err(OrderManagerError::ClientError(e)),
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
            "{} - [{}] Filtered {} duplicate order cancels.",
            type_name::<Self>(),
            self.symbol(),
            order_cancels.len() - filtered_candidates.len()
        );

        filtered_candidates
    }

    /// Submits new orders, adding them to the inflight orders tracker.
    async fn cancel_orders(
        &self,
        order_cancels: &[CandidateCancel],
    ) -> Result<(), OrderManagerError> {
        let filtered_cancels = self.check_inflight_cancels(order_cancels).await;

        if !filtered_cancels.is_empty() {
            let signer = self.signer();
            let rpc_client = self.rpc_client();
            let ixs = self.build_cancel_order_ixs(&filtered_cancels).await;
            match send_transactions(&rpc_client, ixs, &signer, false).await {
                Ok(sigs) => {
                    // add these cancels to the inflight cancels tracker
                    let mut inflight_cancels = self.inflight_cancels_writer().await;
                    inflight_cancels.extend(order_cancels.iter().map(|o| InflightCancel {
                        side: o.side,
                        order_id: o.order_id,
                        client_order_id: o.client_order_id,
                    }));
                    drop(inflight_cancels);
                    for sig in sigs.iter() {
                        info!(
                            "{} - [{}] Sucessfully submitted transaction. Signature: {}.",
                            type_name::<Self>(),
                            self.symbol(),
                            sig
                        );
                    }
                    match self.send_update().await {
                        Ok(()) => {
                            info!(
                                "{} - [{}] Successfully sent orders update.",
                                type_name::<Self>(),
                                self.symbol(),
                            );
                        }
                        Err(e) => {
                            warn!(
                                "{} - [{}] There was an error sending orders update: {:?}",
                                type_name::<Self>(),
                                self.symbol(),
                                e.to_string()
                            );
                        }
                    }
                    Ok(())
                }
                Err(e) => Err(OrderManagerError::ClientError(e)),
            }
        } else {
            Ok(())
        }
    }

    /// Builds cancel order instructions from the given [`CandidateCancel`]s.
    async fn build_cancel_order_ixs(&self, order_cancels: &[CandidateCancel]) -> Vec<Instruction>;

    /// Gets confirmed and inflight orders.
    async fn get_orders(&self) -> Vec<ManagedOrder> {
        let inflight_cancels = self.inflight_cancels_reader().await;
        let managed_orders = self.managed_orders_reader().await;
        let inflight_orders = self.inflight_orders_reader().await;
        let open_orders = self.open_orders_reader().await;

        let mut orders = Vec::new();

        orders.extend(inflight_orders.to_vec());

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
                    client_order_id: order.client_order_id,
                    order_id: order.order_id,
                    side: order.side,
                    ..Default::default()
                }),
            }
        }

        orders
    }

    /// The symbol that this [`OrderManager`] represents.
    fn symbol(&self) -> &str;
}
