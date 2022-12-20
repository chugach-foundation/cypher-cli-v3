use async_trait::async_trait;
use log::info;
use std::any::type_name;
use std::sync::Arc;
use tokio::sync::broadcast::{Receiver, Sender};
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::common::context::{ExecutionContext, GlobalContext, OperationContext};
use crate::common::info::FuturesMarketInfo;
use crate::common::inventory::InventoryManager;
use crate::common::maker::{Maker, MakerError, MakerPulseResult};
use crate::common::oracle::OracleInfo;
use crate::common::orders::{Action, ManagedOrder, OrderManager, OrdersInfo};
use crate::common::strategy::{Strategy, StrategyError};

pub struct FuturesMaker {
    inventory_mngr: Arc<dyn InventoryManager<Input = GlobalContext>>,
    managed_orders: RwLock<OrdersInfo>,
    context: RwLock<ExecutionContext>,
    shutdown_sender: Arc<Sender<bool>>,
    context_sender: Arc<Sender<ExecutionContext>>,
    orders_sender: Arc<Sender<OrdersInfo>>,
    action_sender: Arc<Sender<Action>>,
    order_layers: usize,
    layer_spacing: u16,
    time_in_force: u64,
    symbol: String,
}

impl FuturesMaker {
    pub fn new(
        inventory_mngr: Arc<dyn InventoryManager<Input = GlobalContext>>,
        shutdown_sender: Arc<Sender<bool>>,
        context_sender: Arc<Sender<ExecutionContext>>,
        orders_sender: Arc<Sender<OrdersInfo>>,
        action_sender: Arc<Sender<Action>>,
        order_layers: usize,
        layer_spacing: u16,
        time_in_force: u64,
        symbol: String,
    ) -> Self {
        Self {
            inventory_mngr,
            order_layers,
            layer_spacing,
            time_in_force,
            symbol,
            shutdown_sender,
            context_sender,
            orders_sender,
            action_sender,
            managed_orders: RwLock::new(OrdersInfo::default()),
            context: RwLock::new(ExecutionContext::default()),
        }
    }
}

#[async_trait]
impl Maker for FuturesMaker {
    type Input = ExecutionContext;
    type InventoryManagerInput = GlobalContext;

    fn order_layer_count(&self) -> usize {
        self.order_layers
    }

    fn layer_spacing_bps(&self) -> u16 {
        self.layer_spacing
    }

    fn time_in_force(&self) -> u64 {
        self.time_in_force
    }

    fn action_sender(&self) -> Arc<Sender<Action>> {
        self.action_sender.clone()
    }

    fn orders_receiver(&self) -> Receiver<OrdersInfo> {
        self.orders_sender.subscribe()
    }

    fn context_receiver(&self) -> Receiver<ExecutionContext> {
        self.context_sender.subscribe()
    }

    fn shutdown_receiver(&self) -> Receiver<bool> {
        self.shutdown_sender.subscribe()
    }

    async fn managed_orders_reader(&self) -> RwLockReadGuard<OrdersInfo> {
        self.managed_orders.read().await
    }

    async fn managed_orders_writer(&self) -> RwLockWriteGuard<OrdersInfo> {
        self.managed_orders.write().await
    }

    async fn context_reader(&self) -> RwLockReadGuard<ExecutionContext> {
        self.context.read().await
    }

    async fn context_writer(&self) -> RwLockWriteGuard<ExecutionContext> {
        self.context.write().await
    }

    async fn pulse(&self) -> Result<MakerPulseResult, MakerError> {
        let ctx = self.context_reader().await;
        let orders = self.managed_orders_reader().await;
        let inventory_mngr = self.inventory_manager();

        let spread_info = inventory_mngr.get_spread(ctx.oracle_info.price);
        info!(
            "{} - [{}] Oracle Price: {} - Best bid: {} - Best ask: {}",
            type_name::<Self>(),
            self.symbol,
            spread_info.oracle_price,
            spread_info.bid,
            spread_info.ask,
        );

        let quote_volumes = inventory_mngr.get_quote_volumes(&ctx.global);
        info!(
            "{} - [{}] Current delta: {} - Volumes - Bid: {} - Ask: {}",
            type_name::<Self>(),
            self.symbol,
            quote_volumes.delta,
            quote_volumes.bid_size,
            quote_volumes.ask_size,
        );
        self.update_orders(&quote_volumes, &spread_info, &orders)
            .await
    }

    fn inventory_manager(&self) -> Arc<dyn InventoryManager<Input = GlobalContext>> {
        self.inventory_mngr.clone()
    }

    fn symbol(&self) -> &str {
        self.symbol.as_str()
    }
}
