use async_trait::async_trait;
use log::{info, warn};

use crate::common::{
    context::ExecutionContext,
    hedger::{Hedger, HedgerError, HedgerPulseResult},
    strategy::{Strategy, StrategyError},
};

pub struct SpotHedger {
    symbol: String,
}

impl SpotHedger {
    pub fn new(symbol: String) -> Self {
        Self { symbol }
    }
}

impl Hedger for SpotHedger {
    type Input = ExecutionContext;
    fn pulse(&self, _ctx: &ExecutionContext) -> Result<HedgerPulseResult, HedgerError> {
        info!("[{}] Running spot hedger logic..", self.symbol);

        Ok(HedgerPulseResult::default())
    }
}

#[async_trait]
impl Strategy for SpotHedger
where
    Self: Hedger,
{
    type Input = ExecutionContext;
    type Output = HedgerPulseResult;

    async fn execute(&self, ctx: &ExecutionContext) -> Result<HedgerPulseResult, StrategyError> {
        info!("[{}] Running strategy..", self.symbol);
        match self.pulse(ctx) {
            Ok(res) => {
                info!("Pulse result: {:?}", res);
            }
            Err(e) => {
                warn!("There was an error running hedger pulse. Error: {:?}", e);
            }
        };
        Ok(HedgerPulseResult { size_executed: 0 })
    }
}
