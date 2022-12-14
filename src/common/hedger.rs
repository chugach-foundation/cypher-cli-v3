use async_trait::async_trait;
use thiserror::Error;

/// Represents the result of an hedger's pulse.
#[derive(Default, Debug, Clone)]
pub struct HedgerPulseResult {
    /// The size executed during the pulse.
    /// Negative value means sell size executed, positive value means buy size executed.
    pub size_executed: i128,
}

#[derive(Debug, Error)]
pub enum HedgerError {}

/// Defines shared functionality that different hedgers should implement
#[async_trait]
pub trait Hedger: Send + Sync {
    /// The input type for the hedger.
    type Input;

    /// Triggers a pulse, prompting the hedger to perform it's work cycle.
    fn pulse(&self, input: &Self::Input) -> Result<HedgerPulseResult, HedgerError>;
}
