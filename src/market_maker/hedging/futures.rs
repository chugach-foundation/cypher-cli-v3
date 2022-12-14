use async_trait::async_trait;
use log::info;
use std::any::type_name;

use crate::common::{
    context::ExecutionContext,
    hedger::{Hedger, HedgerError, HedgerPulseResult},
    strategy::{Strategy, StrategyError},
};

pub struct FuturesHedger {
    symbol: String,
}

impl FuturesHedger {
    pub fn new(symbol: String) -> Self {
        Self { symbol }
    }
}

impl Hedger for FuturesHedger {
    type Input = ExecutionContext;

    fn pulse(&self, input: &ExecutionContext) -> Result<HedgerPulseResult, HedgerError> {
        info!(
            "{} - [{}] Running cypher futures hedger logic..",
            type_name::<Self>(),
            self.symbol
        );

        Ok(HedgerPulseResult::default())
    }
}

#[async_trait]
impl Strategy for FuturesHedger
where
    Self: Hedger,
{
    type Input = ExecutionContext;
    type Output = HedgerPulseResult;

    async fn execute(&self, ctx: &ExecutionContext) -> Result<HedgerPulseResult, StrategyError> {
        info!(
            "{} - [{}] Running strategy..",
            type_name::<Self>(),
            self.symbol,
        );
        Ok(HedgerPulseResult { size_executed: 0 })
    }
}
