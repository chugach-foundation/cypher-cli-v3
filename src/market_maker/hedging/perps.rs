use async_trait::async_trait;
use log::info;
use std::any::type_name;

use crate::common::{
    context::ExecutionContext,
    hedger::{Hedger, HedgerError, HedgerPulseResult},
    strategy::{Strategy, StrategyError},
};

pub struct PerpsHedger {
    symbol: String,
}

impl PerpsHedger {
    pub fn new(symbol: String) -> Self {
        Self { symbol }
    }
}

impl Hedger for PerpsHedger {
    type Input = ExecutionContext;
    fn pulse(&self, input: &ExecutionContext) -> Result<HedgerPulseResult, HedgerError> {
        info!(
            "{} - [{}] Running cypher perps hedger logic..",
            type_name::<Self>(),
            self.symbol
        );

        Ok(HedgerPulseResult::default())
    }
}

#[async_trait]
impl Strategy for PerpsHedger
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
        self.pulse(ctx);
        Ok(HedgerPulseResult { size_executed: 0 })
    }
}
