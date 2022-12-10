use crate::common::{
    context::ExecutionContext,
    strategy::{Strategy, StrategyError, StrategyExecutionResult},
};
use async_trait::async_trait;
use log::info;
use std::error;

pub struct RandomBehaviorStrategy {}

#[async_trait]
impl Strategy for RandomBehaviorStrategy {
    async fn execute(
        &self,
        ctx: &ExecutionContext,
    ) -> Result<StrategyExecutionResult, StrategyError> {
        Ok(StrategyExecutionResult {})
    }
}
