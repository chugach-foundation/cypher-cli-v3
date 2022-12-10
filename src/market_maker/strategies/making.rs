use async_trait::async_trait;
use log::{info, warn};
use std::{error, sync::Arc};

use crate::common::{
    context::ExecutionContext,
    maker::Maker,
    strategy::{Strategy, StrategyError, StrategyExecutionResult},
};

pub struct MakingStrategy {
    maker: Arc<dyn Maker>,
}

impl MakingStrategy {
    pub fn new(maker: Arc<dyn Maker>) -> Self {
        Self { maker }
    }
}

#[async_trait]
impl Strategy for MakingStrategy {
    async fn execute(
        &self,
        ctx: &ExecutionContext,
    ) -> Result<StrategyExecutionResult, StrategyError> {
        info!("[MAKING-STRATEGY] Running strategy..");
        Ok(StrategyExecutionResult {})
    }
}
