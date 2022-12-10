use log::info;

use crate::common::context::ExecutionContext;
use crate::market_maker::inventory::InventoryManager;

use crate::common::maker::{Maker, MakerError, MakerPulseResult};

pub struct FuturesMaker {
    symbol: String,
    inventory_mngr: InventoryManager,
}

impl FuturesMaker {
    pub fn new(symbol: String, inventory_mngr: InventoryManager) -> Self {
        Self {
            symbol,
            inventory_mngr,
        }
    }
}

impl Maker for FuturesMaker {
    fn pulse(&self, execution_context: &ExecutionContext) -> Result<MakerPulseResult, MakerError> {
        info!(
            "[CMKR-{}] Running cypher futures market maker logic..",
            self.symbol
        );

        Ok(MakerPulseResult::default())
    }
}
