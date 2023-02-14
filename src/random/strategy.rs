use crate::common::{
    context::ExecutionContext,
    strategy::{Strategy, StrategyError},
};
use async_trait::async_trait;



use super::RandomExecutionResult;

pub struct RandomBehaviorStrategy {}

#[async_trait]
impl Strategy for RandomBehaviorStrategy {
    type Input = ExecutionContext;
    type Output = RandomExecutionResult;

    async fn execute(
        &self,
        _ctx: &ExecutionContext,
    ) -> Result<RandomExecutionResult, StrategyError> {
        Ok(RandomExecutionResult {})
    }
}
