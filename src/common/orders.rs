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
    /// The index of the order in the tracker.
    pub managed_orders_index: usize,
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
                                            warn!("{} - [{}] There was an error canceling orders: {:?}", type_name::<Self>(), symbol, e.to_string());
                                        }
                                    }
                                },
                                Action::CancelOrders(cancels) => {
                                    match self.cancel_orders(&cancels).await {
                                        Ok(()) => (),
                                        Err(e) => {
                                            warn!("{} - [{}] There was an error canceling orders: {:?}", type_name::<Self>(), symbol, e.to_string());
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

    /// Processes an update of the order manager.
    async fn process_update(&self, ctx: &Self::Input) -> Result<(), OrderManagerError> {
        let symbol = self.symbol();
        let mut inflight_cancels_writer = self.inflight_cancels_writer().await;
        let inflight_orders_reader = self.inflight_orders_reader().await;
        let managed_orders_reader = self.managed_orders_reader().await;
        let ctx_open_orders = ctx.get_open_orders();
        let mut inflight_orders_to_remove: Vec<ManagedOrder> = Vec::new();
        let mut inflight_cancels_to_remove: Vec<InflightCancel> = Vec::new();

        info!(
            "{} - [{}] There are {} inflight orders, {} inflight cancels and {} orders on the book.",
            type_name::<Self>(),
            symbol,
            inflight_orders_reader.len(),
            inflight_cancels_writer.len(),
            ctx_open_orders.len()
        );

        for inflight_order in inflight_orders_reader.iter() {
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

        for inflight_cancel in inflight_cancels_writer.iter_mut() {
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
                            "{} - [{}] Inflight cancel confirmed. Side: {:?} - Order ID: {} - Client Order ID: {} - Managed Orders Idx: {}",
                            type_name::<Self>(),
                            symbol,
                            inflight_cancel.side,
                            inflight_cancel.order_id,
                            inflight_cancel.client_order_id,
                            inflight_cancel.managed_orders_index
                        );
                        inflight_cancels_to_remove.push(inflight_cancel.clone());
                    }
                }
            }
        }

        info!(
            "{} - [{}] Found {} inflight orders that have been confirmed..",
            type_name::<Self>(),
            symbol,
            inflight_orders_to_remove.len(),
        );
        info!(
            "{} - [{}] Found {} inflight cancels that have been confirmed..",
            type_name::<Self>(),
            symbol,
            inflight_cancels_to_remove.len(),
        );

        // drop the readers
        drop(inflight_orders_reader);
        drop(managed_orders_reader);

        let mut inflight_orders_writer = self.inflight_orders_writer().await;
        let mut managed_orders_writer = self.managed_orders_writer().await;

        // if this happens we will assume this is after our start-up
        // we'll take any existing on-chain order and add them to managed orders so they end up getting cancelled
        if managed_orders_writer.is_empty() {
            for order in ctx_open_orders.iter() {
                managed_orders_writer.push(ManagedOrder {
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

        inflight_cancels_to_remove
            .sort_by(|a, b| b.managed_orders_index.cmp(&a.managed_orders_index));

        // remove these confirmed cancels from the ones we are tracking
        for order in inflight_cancels_to_remove.iter() {
            let order_idx = inflight_cancels_writer
                .iter()
                .position(|p| {
                    p.client_order_id == order.client_order_id
                        || p.order_id == order.order_id && p.side == order.side
                })
                .unwrap();
            inflight_cancels_writer.remove(order_idx);

            match managed_orders_writer.iter().position(|p| {
                p.client_order_id == order.client_order_id
                    || p.order_id == order.order_id && p.side == order.side
            }) {
                Some(i) => {
                    managed_orders_writer.remove(i);
                }
                None => continue,
            }
        }

        // remove these confirmed orders from the ones we are tracking
        for order in inflight_orders_to_remove.iter() {
            let order_idx = inflight_orders_writer
                .iter()
                .position(|p| p.client_order_id == order.client_order_id)
                .unwrap();
            inflight_orders_writer.remove(order_idx);
            managed_orders_writer.push(order.clone());
        }

        info!(
            "{} - [{}] There are {} managed open orders..",
            type_name::<Self>(),
            symbol,
            managed_orders_writer.len()
        );

        // drop the writers
        drop(inflight_orders_writer);
        drop(inflight_cancels_writer);
        drop(managed_orders_writer);

        let mut open_orders_writer = self.open_orders_writer().await;
        *open_orders_writer = ctx_open_orders.to_vec();

        info!(
            "{} - [{}] There are {} confirmed open orders..",
            type_name::<Self>(),
            symbol,
            ctx_open_orders.len()
        );

        Ok(())
    }

    async fn check_inflight_orders(&self) -> bool {
        let inflight_orders_reader = self.inflight_orders_reader().await;
        if inflight_orders_reader.is_empty() {
            false
        } else {
            true
        }
    }

    /// Submits new orders, adding them to the inflight orders tracker.
    async fn place_orders(
        &self,
        order_placements: &[CandidatePlacement],
    ) -> Result<(), OrderManagerError> {
        if self.check_inflight_orders().await {
            Ok(())
        } else {
            let signer = self.signer();
            let rpc_client = self.rpc_client();
            let (ixs, managed_orders) = self.build_new_order_ixs(order_placements).await;
            match send_transactions(&rpc_client, ixs, &signer, false).await {
                Ok(sigs) => {
                    // add these submitted orders to the inflight orders tracker
                    let mut inflight_orders_writer = self.inflight_orders_writer().await;
                    inflight_orders_writer.extend(managed_orders);
                    for sig in sigs.iter() {
                        info!(
                            "{} - [{}] Sucessfully submitted transaction. Signature: {}.",
                            type_name::<Self>(),
                            self.symbol(),
                            sig
                        );
                    }
                    Ok(())
                }
                Err(e) => Err(OrderManagerError::ClientError(e)),
            }
        }
    }

    /// Builds cancel order instructions from the given [`CandidatePlacement`]s.
    async fn build_new_order_ixs(
        &self,
        order_placements: &[CandidatePlacement],
    ) -> (Vec<Instruction>, Vec<ManagedOrder>);

    async fn check_inflight_cancels(&self) -> bool {
        let inflight_cancels_reader = self.inflight_cancels_reader().await;
        if inflight_cancels_reader.is_empty() {
            false
        } else {
            true
        }
    }

    /// Submits new orders, adding them to the inflight orders tracker.
    async fn cancel_orders(
        &self,
        order_cancels: &[CandidateCancel],
    ) -> Result<(), OrderManagerError> {
        if self.check_inflight_cancels().await {
            Ok(())
        } else {
            let signer = self.signer();
            let rpc_client = self.rpc_client();
            let ixs = self.build_cancel_order_ixs(order_cancels).await;
            match send_transactions(&rpc_client, ixs, &signer, false).await {
                Ok(sigs) => {
                    // add these cancels to the inflight cancels tracker
                    let mut inflight_cancels_writer = self.inflight_cancels_writer().await;
                    inflight_cancels_writer.extend(order_cancels.iter().map(|o| InflightCancel {
                        side: o.side,
                        order_id: o.order_id,
                        client_order_id: o.client_order_id,
                        managed_orders_index: usize::MAX,
                    }));
                    for sig in sigs.iter() {
                        info!(
                            "{} - [{}] Sucessfully submitted transaction. Signature: {}.",
                            type_name::<Self>(),
                            self.symbol(),
                            sig
                        );
                    }
                    Ok(())
                }
                Err(e) => Err(OrderManagerError::ClientError(e)),
            }
        }
    }

    /// Builds cancel order instructions from the given [`CandidateCancel`]s.
    async fn build_cancel_order_ixs(&self, order_cancels: &[CandidateCancel]) -> Vec<Instruction>;

    /// Gets confirmed and inflight orders.
    async fn get_orders(&self) -> Vec<ManagedOrder> {
        let inflight_cancels_reader = self.inflight_cancels_reader().await;
        let inflight_cancels = inflight_cancels_reader.to_vec();

        let managed_orders_reader = self.managed_orders_reader().await;
        let inflight_orders_reader = self.inflight_orders_reader().await;
        let open_orders_reader = self.open_orders_reader().await;

        let mut orders = Vec::new();

        for order in managed_orders_reader.iter() {
            match inflight_cancels.iter().find(|c| {
                c.order_id == order.order_id || c.client_order_id == order.client_order_id
            }) {
                Some(_) => (),
                None => orders.push(order.clone()),
            }
        }

        for order in open_orders_reader.iter() {
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
