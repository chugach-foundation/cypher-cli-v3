use log::info;

use crate::common::context::ExecutionContext;
use crate::market_maker::inventory::InventoryManager;

use crate::common::maker::{Maker, MakerError, MakerPulseResult};

pub struct SpotMaker {
    symbol: String,
    inventory_mngr: InventoryManager,
}

impl SpotMaker {
    pub fn new(symbol: String, inventory_mngr: InventoryManager) -> Self {
        Self {
            symbol,
            inventory_mngr,
        }
    }
}

impl Maker for SpotMaker {
    fn pulse(&self, execution_context: &ExecutionContext) -> Result<MakerPulseResult, MakerError> {
        info!(
            "[SPOTMKR-{}] Running spot market maker logic..",
            self.symbol
        );

        Ok(MakerPulseResult::default())
    }
}
