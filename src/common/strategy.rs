use async_trait::async_trait;
use thiserror::Error;

use super::{context::ExecutionContext, hedger::HedgerError, maker::MakerError};

#[derive(Error, Debug)]
pub enum StrategyError {}

/// The result of a strategy execution tick.
pub struct StrategyExecutionResult {}

/// A trait that represents shared functionality for strategies.
#[async_trait]
pub trait Strategy: Send + Sync {
    /// Executes the strategy.
    async fn execute(
        &self,
        ctx: &ExecutionContext,
    ) -> Result<StrategyExecutionResult, StrategyError>;
}
