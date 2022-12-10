use log::info;

use crate::common::hedger::{Hedger, HedgerError, HedgerPulseResult};

pub struct SpotHedger {
    symbol: String,
}

impl SpotHedger {
    pub fn new(symbol: String) -> Self {
        Self { symbol }
    }
}

impl Hedger for SpotHedger {
    fn pulse(&self) -> Result<HedgerPulseResult, HedgerError> {
        info!("[SPOTHDGR-{}] Running spot hedger logic..", self.symbol);

        Ok(HedgerPulseResult::default())
    }
}
