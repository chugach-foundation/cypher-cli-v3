use async_trait::async_trait;
use log::info;
use std::any::type_name;
use std::sync::Arc;
use tokio::sync::broadcast::{Receiver, Sender};
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::common::context::{ExecutionContext, GlobalContext, OperationContext};
use crate::common::info::SpotMarketInfo;
use crate::common::inventory::InventoryManager;
use crate::common::oracle::OracleInfo;
use crate::common::orders::{Action, ManagedOrder, OrderManager};
use crate::common::strategy::{Strategy, StrategyError};

use crate::common::maker::{Maker, MakerError, MakerPulseResult};

pub struct SpotMaker {
    inventory_mngr: Arc<dyn InventoryManager<Input = GlobalContext>>,
    managed_orders: RwLock<Vec<ManagedOrder>>,
    context: RwLock<ExecutionContext>,
    shutdown_sender: Arc<Sender<bool>>,
    context_sender: Arc<Sender<ExecutionContext>>,
    orders_sender: Arc<Sender<Vec<ManagedOrder>>>,
    action_sender: Arc<Sender<Action>>,
    order_layers: usize,
    layer_spacing: u16,
    symbol: String,
}

impl SpotMaker {
    pub fn new(
        inventory_mngr: Arc<dyn InventoryManager<Input = GlobalContext>>,
        shutdown_sender: Arc<Sender<bool>>,
        context_sender: Arc<Sender<ExecutionContext>>,
        orders_sender: Arc<Sender<Vec<ManagedOrder>>>,
        action_sender: Arc<Sender<Action>>,
        order_layers: usize,
        layer_spacing: u16,
        symbol: String,
    ) -> Self {
        Self {
            inventory_mngr,
            order_layers,
            layer_spacing,
            symbol,
            shutdown_sender,
            context_sender,
            orders_sender,
            action_sender,
            managed_orders: RwLock::new(Vec::new()),
            context: RwLock::new(ExecutionContext::default()),
        }
    }
}

#[async_trait]
impl Maker for SpotMaker {
    type Input = ExecutionContext;
    type InventoryManagerInput = GlobalContext;

    fn order_layer_count(&self) -> usize {
        self.order_layers
    }

    fn layer_spacing_bps(&self) -> u16 {
        self.layer_spacing
    }

    fn time_in_force(&self) -> u64 {
        60 // let's simply default to 60
    }

    fn action_sender(&self) -> Arc<Sender<Action>> {
        self.action_sender.clone()
    }

    fn orders_receiver(&self) -> Receiver<Vec<ManagedOrder>> {
        self.orders_sender.subscribe()
    }

    fn context_receiver(&self) -> Receiver<ExecutionContext> {
        self.context_sender.subscribe()
    }

    fn shutdown_receiver(&self) -> Receiver<bool> {
        self.shutdown_sender.subscribe()
    }

    async fn managed_orders_reader(&self) -> RwLockReadGuard<Vec<ManagedOrder>> {
        self.managed_orders.read().await
    }

    async fn managed_orders_writer(&self) -> RwLockWriteGuard<Vec<ManagedOrder>> {
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
