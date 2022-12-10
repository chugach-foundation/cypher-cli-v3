use log::info;

use crate::common::hedger::{Hedger, HedgerError, HedgerPulseResult};

pub struct PerpsHedger {
    symbol: String,
}

impl PerpsHedger {
    pub fn new(symbol: String) -> Self {
        Self { symbol }
    }
}

impl Hedger for PerpsHedger {
    fn pulse(&self) -> Result<HedgerPulseResult, HedgerError> {
        info!("[CMKR-{}] Running cypher perps hedger logic..", self.symbol);

        Ok(HedgerPulseResult::default())
    }
}
