use log::info;

use crate::common::hedger::{Hedger, HedgerError, HedgerPulseResult};

pub struct FuturesHedger {
    symbol: String,
}

impl FuturesHedger {
    pub fn new(symbol: String) -> Self {
        Self { symbol }
    }
}

impl Hedger for FuturesHedger {
    fn pulse(&self) -> Result<HedgerPulseResult, HedgerError> {
        info!(
            "[CMKR-{}] Running cypher futures hedger logic..",
            self.symbol
        );

        Ok(HedgerPulseResult::default())
    }
}
