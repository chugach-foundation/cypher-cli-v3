pub mod cypher;
pub mod serum;

/// Represents the result of a maker's pulse.
pub struct MakerPulseResult {
    /// Number of new orders submitted.
    pub num_new_orders: usize,
    /// Number of cancelled orders.
    pub num_cancelled_orders: usize,
}

/// Defines shared functionality that different makers should implement
pub trait Maker {
    /// Triggers a pulse, prompting the maker to perform it's work cycle.
    fn pulse(&self) -> MakerPulseResult;
}
