use std::sync::Arc;

use anchor_spl::dex::serum_dex::state::MarketState;
use cypher_client::{FuturesMarket, PerpetualMarket};
use solana_sdk::{pubkey::Pubkey, signature::Keypair};

#[derive(Clone)]
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

#[derive(Debug, Clone)]
pub struct UserInfo {
    /// The user's master account pubkey.
    pub master_account: Pubkey,
    /// The user's sub account pubkey.
    pub sub_account: Pubkey,
    /// The user's clearing account pubkey.
    pub clearing: Pubkey,
    /// The user's signer keypair.
    pub signer: Arc<Keypair>,
}

impl Default for UserInfo {
    fn default() -> Self {
        Self {
            master_account: Pubkey::default(),
            sub_account: Pubkey::default(),
            clearing: Pubkey::default(),
            signer: Arc::new(Keypair::new()),
        }
    }
}

#[derive(Default, Debug, Clone)]
pub struct MarketMetadata {
    /// The decimals of the underlying asset/position.
    pub decimals: u8,
    /// The cache index for this market.
    pub cache_index: u16,
    /// The base multiplier or base lot size of the market.
    pub base_multiplier: u64,
    /// The quote multiplier or quote lot size of the market.
    pub quote_multiplier: u64,
}

#[derive(Default, Clone)]
pub struct PerpMarketInfo {
    /// The perpetual market state.
    pub state: PerpetualMarket,
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
    /// The quote pool account pubkey.
    pub quote_pool: Pubkey,
    /// The quote pool node accounts pubkeys.
    pub quote_pool_nodes: Vec<Pubkey>,
}

impl PerpMarketInfo {
    pub fn to_accounts_vec(&self) -> Vec<Pubkey> {
        let mut accounts = vec![
            self.market,
            self.orderbook,
            self.bids,
            self.asks,
            self.event_queue,
            self.orders,
            self.quote_pool,
        ];
        accounts.extend(self.quote_pool_nodes.clone());
        accounts
    }
}

#[derive(Default, Clone)]
pub struct FuturesMarketInfo {
    /// The futures market state.
    pub state: FuturesMarket,
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
    /// The quote pool account pubkey.
    pub quote_pool: Pubkey,
    /// The quote pool node accounts pubkeys.
    pub quote_pool_nodes: Vec<Pubkey>,
}

impl FuturesMarketInfo {
    pub fn to_accounts_vec(&self) -> Vec<Pubkey> {
        let mut accounts = vec![
            self.market,
            self.orderbook,
            self.bids,
            self.asks,
            self.event_queue,
            self.orders,
            self.quote_pool,
            self.price_history,
        ];
        accounts.extend(self.quote_pool_nodes.clone());
        accounts
    }
}

#[derive(Clone)]
pub struct SpotMarketInfo {
    /// The spot market state.
    pub state: MarketState,
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
    /// The base mint account pubkey.
    pub asset_mint: Pubkey,
    /// The base vault account pubkey.
    pub asset_vault: Pubkey,
    /// The base vault signer pubkey.
    pub asset_vault_signer: Pubkey,
    /// The quote vault account pubkey.
    pub quote_vault: Pubkey,
    /// The quote vault signer pubkey.
    pub quote_vault_signer: Pubkey,
    /// The open orders account pubkey.
    pub open_orders: Pubkey,
    /// The quote pool account pubkey.
    pub quote_pool: Pubkey,
    /// The dex coin vault account pubkey.
    pub dex_coin_vault: Pubkey,
    /// The dex price coin vault account pubkey.
    pub dex_pc_vault: Pubkey,
    /// The dex vault signer pubkey.
    pub dex_vault_signer: Pubkey,
    /// The quote pool node accounts pubkeys.
    pub quote_pool_nodes: Vec<Pubkey>,
    /// The asset pool account pubkey.
    pub asset_pool: Pubkey,
    /// The asset pool node accounts pubkeys.
    pub asset_pool_nodes: Vec<Pubkey>,
}

impl SpotMarketInfo {
    pub fn to_accounts_vec(&self) -> Vec<Pubkey> {
        let mut accounts = vec![
            self.market,
            self.bids,
            self.asks,
            self.event_queue,
            self.open_orders,
            self.quote_pool,
            self.asset_pool,
        ];
        accounts.extend(self.quote_pool_nodes.clone());
        accounts.extend(self.asset_pool_nodes.clone());
        accounts
    }
}

impl Default for SpotMarketInfo {
    fn default() -> Self {
        Self {
            state: MarketState {
                account_flags: u64::default(),
                own_address: [0u64; 4],
                vault_signer_nonce: u64::default(),
                coin_mint: [0u64; 4],
                pc_mint: [0u64; 4],
                coin_vault: [0u64; 4],
                coin_deposits_total: u64::default(),
                coin_fees_accrued: u64::default(),
                pc_vault: [0u64; 4],
                pc_deposits_total: u64::default(),
                pc_fees_accrued: u64::default(),
                pc_dust_threshold: u64::default(),
                req_q: [0u64; 4],
                event_q: [0u64; 4],
                bids: [0u64; 4],
                asks: [0u64; 4],
                coin_lot_size: u64::default(),
                pc_lot_size: u64::default(),
                fee_rate_bps: u64::default(),
                referrer_rebates_accrued: u64::default(),
            },
            market: Pubkey::default(),
            bids: Pubkey::default(),
            asks: Pubkey::default(),
            event_queue: Pubkey::default(),
            request_queue: Pubkey::default(),
            asset_mint: Pubkey::default(),
            asset_vault: Pubkey::default(),
            asset_vault_signer: Pubkey::default(),
            quote_vault: Pubkey::default(),
            quote_vault_signer: Pubkey::default(),
            open_orders: Pubkey::default(),
            dex_coin_vault: Pubkey::default(),
            dex_pc_vault: Pubkey::default(),
            dex_vault_signer: Pubkey::default(),
            quote_pool: Pubkey::default(),
            quote_pool_nodes: Vec::new(),
            asset_pool: Pubkey::default(),
            asset_pool_nodes: Vec::new(),
        }
    }
}
