use fixed::types::I80F48;

/// Reoresents information about desired quote volumes.
#[derive(Debug, Default)]
pub struct QuoteVolumes {
    pub delta: I80F48,
    pub bid_size: I80F48,
    pub ask_size: I80F48,
}

/// Represents information about a spread.
#[derive(Debug, Default)]
pub struct SpreadInfo {
    pub oracle_price: I80F48,
    pub bid: I80F48,
    pub ask: I80F48,
}

/// Defines shared functionality that different inventory managers should implement.
pub trait InventoryManager: Send + Sync {
    /// The input type for the [`InventoryManager`].
    type Input: Clone + Send + Sync;

    /// Gets the current delta.
    fn get_delta(&self, input: &Self::Input) -> I80F48;

    /// Gets the desired quote size according to [`InventoryManager`] configuration and the user's absolute delta.
    fn get_quote_size(&self, absolute_delta: I80F48) -> I80F48;

    /// Gets the desired quote volumes according to [`InventoryManager`] configuration and current delta.
    fn get_quote_volumes(&self, input: &Self::Input) -> QuoteVolumes;

    /// Gets the desired spread according to the [`InventoryManager`] configuration.
    fn get_spread(&self, oracle_price: I80F48) -> SpreadInfo;
}
