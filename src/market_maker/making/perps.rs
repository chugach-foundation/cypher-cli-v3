use log::info;

use crate::common::context::ExecutionContext;
use crate::market_maker::inventory::InventoryManager;

use crate::common::maker::{Maker, MakerError, MakerPulseResult};

pub struct PerpsMaker {
    symbol: String,
    inventory_mngr: InventoryManager,
}

impl PerpsMaker {
    pub fn new(symbol: String, inventory_mngr: InventoryManager) -> Self {
        Self {
            symbol,
            inventory_mngr,
        }
    }
}

impl Maker for PerpsMaker {
    fn pulse(&self, execution_context: &ExecutionContext) -> Result<MakerPulseResult, MakerError> {
        info!(
            "[CMKR-{}] Running cypher perps market maker logic..",
            self.symbol
        );

        Ok(MakerPulseResult::default())
    }
}
