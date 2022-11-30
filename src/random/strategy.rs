use crate::{
    common::strategy::{Strategy, StrategyExecutionResult},
    context::ExecutionContext,
};
use log::info;
use std::error;

pub struct RandomBehaviorStrategy {}

#[async_trait(?Send)]
impl Strategy for RandomBehaviorStrategy {
    async fn execute(
        &self,
        ctx: &ExecutionContext,
    ) -> Result<StrategyExecutionResult, Box<dyn error::Error>> {
        Ok(StrategyExecutionResult {})
    }
}
