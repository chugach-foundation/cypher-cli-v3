pub mod cypher;
pub mod serum;

/// Represents the result of an hedger's pulse.
pub struct HedgerPulseResult {
    /// The size executed during the pulse.
    /// Negative value means sell size executed, positive value means buy size executed.
    pub size_executed: i128,
}

/// Defines shared functionality that different hedgers should implement
pub trait Hedger {
    /// Triggers a pulse, prompting the hedger to perform it's work cycle.
    fn pulse(&self) -> HedgerPulseResult;
}
