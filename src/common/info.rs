use solana_sdk::pubkey::Pubkey;

#[derive(Debug, Clone, Copy)]
pub enum Accounts {
    Futures(FuturesMarketInfo),
    Perpetuals(PerpMarketInfo),
    Spot(SpotMarketInfo),
}

impl Default for Accounts {
    fn default() -> Self {
        Self::Futures(FuturesMarketInfo::default())
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PerpMarketInfo {
    /// The market account pubkey.
    pub market: Pubkey,
    /// The orderbook account pubkey.
    pub orderbook: Pubkey,
    /// The bids account pubkey.
    pub bids: Pubkey,
    /// The asks account pubkey.
    pub asks: Pubkey,
    /// The event queue account pubkey.
    pub event_queue: Pubkey,
    /// The orders account pubkey.
    pub orders: Pubkey,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FuturesMarketInfo {
    /// The market account pubkey.
    pub market: Pubkey,
    /// The orderbook account pubkey.
    pub orderbook: Pubkey,
    /// The bids account pubkey.
    pub bids: Pubkey,
    /// The asks account pubkey.
    pub asks: Pubkey,
    /// The event queue account pubkey.
    pub event_queue: Pubkey,
    /// The price history account pubkey.
    pub price_history: Pubkey,
    /// The orders account pubkey.
    pub orders: Pubkey,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SpotMarketInfo {
    /// The market account pubkey.
    pub market: Pubkey,
    /// The bids account pubkey.
    pub bids: Pubkey,
    /// The asks account pubkey.
    pub asks: Pubkey,
    /// The event queue account pubkey.
    pub event_queue: Pubkey,
    /// The request queue account pubkey.
    pub request_queue: Pubkey,
    /// The base vault account pubkey.
    pub base_vault: Pubkey,
    /// The quote vault account pubkey.
    pub quote_vault: Pubkey,
    /// The dex vault signer pubkey.
    pub vault_signer: Pubkey,
    /// The open orders account pubkey.
    pub open_orders: Pubkey,
}
