use async_trait::async_trait;
use log::info;

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
    fn pulse(&self, ctx: &ExecutionContext) -> Result<HedgerPulseResult, HedgerError> {
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
        self.pulse(ctx);
        Ok(HedgerPulseResult { size_executed: 0 })
    }
}
