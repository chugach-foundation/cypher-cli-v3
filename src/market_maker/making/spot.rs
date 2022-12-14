use async_trait::async_trait;
use log::info;
use std::any::type_name;
use std::sync::Arc;
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::common::context::{ExecutionContext, GlobalContext, OperationContext};
use crate::common::info::SpotMarketInfo;
use crate::common::inventory::InventoryManager;
use crate::common::oracle::OracleInfo;
use crate::common::orders::OrderManager;
use crate::common::strategy::{Strategy, StrategyError};

use crate::common::maker::{Maker, MakerError, MakerPulseResult};

pub struct SpotMaker {
    inventory_mngr: Arc<dyn InventoryManager<Input = GlobalContext>>,
    order_mngr: Arc<dyn OrderManager<Input = OperationContext>>,
    order_layers: usize,
    layer_spacing: u16,
    symbol: String,
}

impl SpotMaker {
    pub fn new(
        inventory_mngr: Arc<dyn InventoryManager<Input = GlobalContext>>,
        order_mngr: Arc<dyn OrderManager<Input = OperationContext>>,
        order_layers: usize,
        layer_spacing: u16,
        symbol: String,
    ) -> Self {
        Self {
            inventory_mngr,
            order_mngr,
            order_layers,
            layer_spacing,
            symbol,
        }
    }
}

#[async_trait]
impl Maker for SpotMaker {
    type Input = ExecutionContext;
    type InventoryManagerInput = GlobalContext;
    type OrderManagerInput = OperationContext;

    fn order_layer_count(&self) -> usize {
        self.order_layers
    }

    fn layer_spacing_bps(&self) -> u16 {
        self.layer_spacing
    }

    fn time_in_force(&self) -> u64 {
        60 // let's simply default to 60
    }

    async fn pulse(
        &self,
        execution_context: &ExecutionContext,
    ) -> Result<MakerPulseResult, MakerError> {
        let inventory_mngr = self.inventory_manager();

        let spread_info = inventory_mngr.get_spread(execution_context.oracle_info.price);
        info!(
            "{} - [{}] Oracle Price: {} - Best bid: {} - Best ask: {}",
            type_name::<Self>(),
            self.symbol,
            spread_info.oracle_price,
            spread_info.bid,
            spread_info.ask,
        );

        let quote_volumes = inventory_mngr.get_quote_volumes(&execution_context.global);
        info!(
            "{} - [{}] Current delta: {} - Volumes - Bid: {} - Ask: {}",
            type_name::<Self>(),
            self.symbol,
            quote_volumes.delta,
            quote_volumes.bid_size,
            quote_volumes.ask_size,
        );

        self.update_orders(&quote_volumes, &spread_info).await
    }

    fn inventory_manager(&self) -> Arc<dyn InventoryManager<Input = GlobalContext>> {
        self.inventory_mngr.clone()
    }

    fn order_manager(&self) -> Arc<dyn OrderManager<Input = OperationContext>> {
        self.order_mngr.clone()
    }

    fn symbol(&self) -> &str {
        self.symbol.as_str()
    }
}

#[async_trait]
impl Strategy for SpotMaker
where
    Self: Maker,
{
    type Input = ExecutionContext;
    type Output = MakerPulseResult;

    async fn execute(&self, ctx: &ExecutionContext) -> Result<MakerPulseResult, StrategyError> {
        info!(
            "{} - [{}] Running strategy..",
            type_name::<Self>(),
            self.symbol
        );
        match self.pulse(ctx).await {
            Ok(o) => Ok(o),
            Err(e) => Err(StrategyError::MakerError(e)),
        }
    }
}
