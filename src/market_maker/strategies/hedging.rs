use async_trait::async_trait;
use log::{info, warn};
use std::{error, sync::Arc};

use crate::common::{
    context::ExecutionContext,
    hedger::Hedger,
    maker::Maker,
    strategy::{Strategy, StrategyError, StrategyExecutionResult},
};

pub struct HedgingStrategy {
    hedger: Arc<dyn Hedger>,
}

impl HedgingStrategy {
    pub fn new(hedger: Arc<dyn Hedger>) -> Self {
        Self { hedger }
    }
}

#[async_trait]
impl Strategy for HedgingStrategy {
    async fn execute(
        &self,
        ctx: &ExecutionContext,
    ) -> Result<StrategyExecutionResult, StrategyError> {
        info!("[HEDGING-STRATEGY] Running strategy..");
        Ok(StrategyExecutionResult {})
    }
}
