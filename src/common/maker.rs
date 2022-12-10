use async_trait::async_trait;
use thiserror::Error;

use super::context::ExecutionContext;

/// Represents the result of a maker's pulse.
#[derive(Default, Debug)]
pub struct MakerPulseResult {
    /// Number of new orders submitted.
    pub num_new_orders: usize,
    /// Number of cancelled orders.
    pub num_cancelled_orders: usize,
}

#[derive(Debug, Error)]
pub enum MakerError {}

/// Defines shared functionality that different makers should implement
#[async_trait]
pub trait Maker: Send + Sync {
    /// Triggers a pulse, prompting the maker to perform it's work cycle.
    fn pulse(&self, execution_context: &ExecutionContext) -> Result<MakerPulseResult, MakerError>;
}
