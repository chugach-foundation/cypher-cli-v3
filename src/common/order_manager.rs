use async_trait::async_trait;
use cypher_utils::contexts::Order;
use fixed::types::I80F48;
use log::info;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{RwLockReadGuard, RwLockWriteGuard};

use super::{
    context::OrdersContext,
    orders::{InflightCancel, ManagedOrder},
    Identifier,
};

#[async_trait]
pub trait OrderManager: Send + Sync + Identifier {
    /// Gets the [`ManagedOrder`]s reader.
    async fn managed_orders_reader(&self) -> RwLockReadGuard<Vec<ManagedOrder>>;

    /// Gets the [`ManagedOrder`]s writer.
    async fn managed_orders_writer(&self) -> RwLockWriteGuard<Vec<ManagedOrder>>;

    /// Gets the open [`Order`]s reader.
    async fn open_orders_reader(&self) -> RwLockReadGuard<Vec<Order>>;

    /// Gets the open [`Order`]s writer.
    async fn open_orders_writer(&self) -> RwLockWriteGuard<Vec<Order>>;

    /// Gets the inflight [`ManagedOrder`]s reader.
    async fn inflight_orders_reader(&self) -> RwLockReadGuard<Vec<ManagedOrder>>;

    /// Gets the inflight [`ManagedOrder`]s writer.
    async fn inflight_orders_writer(&self) -> RwLockWriteGuard<Vec<ManagedOrder>>;

    /// Gets the [`InflightCancel`]s reader.
    async fn inflight_cancels_reader(&self) -> RwLockReadGuard<Vec<InflightCancel>>;

    /// Gets the [`InflightCancel`]s writer.
    async fn inflight_cancels_writer(&self) -> RwLockWriteGuard<Vec<InflightCancel>>;

    /// Updates [`InflightCancel`]s.
    async fn update_inflight_cancels(&self, ctx: &dyn OrdersContext) {
        let symbol = self.symbol();
        let cur_ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let mut inflight_cancels = self.inflight_cancels_writer().await;
        let mut inflight_cancels_to_remove: Vec<InflightCancel> = Vec::new();
        let ctx_open_orders = ctx.get_open_orders();
        info!(
            "[{}] There are {} inflight cancels and {} orders on the book.",
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
                    if !inflight_cancels_to_remove.contains(inflight_cancel) {
                        info!(
                            "[{}] Inflight cancel confirmed. Side: {:?} - Order ID: {} - Client Order ID: {}",
                            symbol,
                            inflight_cancel.side,
                            inflight_cancel.order_id,
                            inflight_cancel.client_order_id
                        );
                        inflight_cancels_to_remove.push(inflight_cancel.clone());
                    }
                }
            }
            // we have to admit the possibility that something might have gone wrong with an update
            // or an inflight cancel might not have materialized and it can get us stuck in a loop
            // TODO: instead of hardcoded value try changing this to a configurable param
            if inflight_cancel.submitted_at + 15 < cur_ts.as_secs() {
                inflight_cancels_to_remove.push(inflight_cancel.clone());
            }
        }

        info!(
            "[{}] Found {} inflight cancels that have been confirmed..",
            symbol,
            inflight_cancels_to_remove.len(),
        );

        if !inflight_cancels_to_remove.is_empty() {
            let mut managed_orders = self.managed_orders_writer().await;
            // remove these confirmed cancels from the ones we are tracking
            for order in inflight_cancels_to_remove.iter() {
                let order_idx = inflight_cancels.iter().position(|p| {
                    p.client_order_id == order.client_order_id
                        || p.order_id == order.order_id && p.side == order.side
                });
                match order_idx {
                    Some(idx) => {
                        inflight_cancels.remove(idx);
                    }
                    None => continue,
                };
                match managed_orders.iter().position(|p| {
                    p.client_order_id == order.client_order_id
                        || p.order_id == order.order_id && p.side == order.side
                }) {
                    Some(i) => {
                        managed_orders.remove(i);
                    }
                    None => continue,
                };
            }
        }
    }

    /// Updates inflight [`ManagedOrder`]s.
    async fn update_inflight_orders(&self, ctx: &dyn OrdersContext) {
        let symbol = self.symbol();
        let cur_ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let mut inflight_orders = self.inflight_orders_writer().await;
        let mut inflight_orders_to_move: Vec<ManagedOrder> = Vec::new();
        let mut inflight_orders_to_remove: Vec<ManagedOrder> = Vec::new();
        let ctx_open_orders = ctx.get_open_orders();

        for inflight_order in inflight_orders.iter() {
            // check if any of our "inflight orders" have now been confirmed
            // and if so, we should remove them from our tracking
            if let Some(o) = ctx_open_orders
                .iter()
                .find(|p| p.client_order_id == inflight_order.client_order_id)
            {
                let mut order = inflight_order.clone();
                order.order_id = o.order_id;
                if !inflight_orders_to_move.contains(&order) {
                    info!(
                        "[{}] Inflight order confirmed. Side: {:?} - Price: {} - Size: {} - Layer: {} - Order ID: {} - Client Order ID: {}",
                        symbol,
                        inflight_order.side,
                        inflight_order.price,
                        inflight_order.base_quantity,
                        inflight_order.layer,
                        inflight_order.order_id,
                        inflight_order.client_order_id,
                    );
                    inflight_orders_to_move.push(order);
                }
            };
            // we have to admit the possibility that something might have gone wrong with an update
            // or an inflight order might not have materialized and it can get us stuck in a loop
            // TODO: instead of hardcoded value try changing this to a configurable param
            if inflight_order.submitted_at + 15 < cur_ts.as_secs() {
                inflight_orders_to_remove.push(inflight_order.clone());
            }
        }

        if !inflight_orders_to_move.is_empty() {
            info!(
                "[{}] Found {} inflight orders that have been confirmed..",
                symbol,
                inflight_orders_to_move.len(),
            );
            let mut managed_orders = self.managed_orders_writer().await;

            // remove these confirmed orders from the ones we are tracking
            for order in inflight_orders_to_move.iter() {
                let order_idx = inflight_orders
                    .iter()
                    .position(|p| p.client_order_id == order.client_order_id);

                match order_idx {
                    Some(idx) => {
                        inflight_orders.remove(idx);
                        managed_orders.push(order.clone());
                    }
                    None => continue,
                }
            }
        }

        if !inflight_orders_to_remove.is_empty() {
            info!(
                "[{}] Found {} inflight orders that have taken too long to confirm..",
                symbol,
                inflight_orders_to_remove.len(),
            );

            // remove these confirmed orders from the ones we are tracking
            for order in inflight_orders_to_remove.iter() {
                let order_idx = inflight_orders
                    .iter()
                    .position(|p| p.client_order_id == order.client_order_id);

                match order_idx {
                    Some(idx) => {
                        inflight_orders.remove(idx);
                    }
                    None => continue,
                }
            }
        }
    }

    /// Processes an update of the [`OrderManager`].
    async fn process_update(&self, ctx: &dyn OrdersContext) {
        let symbol = self.symbol();

        info!("[{}] Processing update..", symbol);

        // update our tracking of inflight cancels
        self.update_inflight_cancels(ctx).await;
        // update our tracking of inflight orders
        self.update_inflight_orders(ctx).await;

        let mut managed_orders = self.managed_orders_writer().await;
        let ctx_open_orders = ctx.get_open_orders();

        // if this happens we will assume this is after our start-up
        // we'll take any existing on-chain order and add them to managed orders so they end up getting cancelled
        if managed_orders.is_empty() {
            info!(
                "[{}] There are no managed orders, adding {} confirmed open orders..",
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
                    submitted_at: u64::MIN,
                    client_order_id: order.client_order_id,
                    order_id: order.order_id,
                    side: order.side,
                    layer: usize::MAX,
                });
            }
        } else {
            // otherwise, let's see if we have any order in our state that doesn't seem to exist on-chain
            let mut order_idxs = Vec::new();
            for (idx, order) in managed_orders.iter().enumerate() {
                match ctx_open_orders.iter().position(|o| {
                    (o.order_id == order.order_id || o.client_order_id == order.client_order_id)
                        && o.side == order.side
                }) {
                    Some(_) => (),
                    None => order_idxs.push(idx),
                }
            }

            order_idxs.sort_by(|a, b| b.cmp(a));

            for idx in order_idxs.iter() {
                managed_orders.remove(*idx);
            }
        }

        let mut open_orders = self.open_orders_writer().await;
        *open_orders = ctx_open_orders.to_vec();

        info!(
            "[{}] There are {} confirmed open orders..",
            symbol,
            ctx_open_orders.len()
        );
    }
}
