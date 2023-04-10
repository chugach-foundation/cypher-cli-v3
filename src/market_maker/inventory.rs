#![allow(dead_code)]
use {fixed::types::I80F48, log::info, solana_sdk::pubkey::Pubkey};

use crate::common::{
    context::GlobalContext,
    inventory::{InventoryManager, QuoteVolumes, SpreadInfo},
    Identifier,
};

pub struct ShapeFunctionInventoryManager {
    decimals: u8,
    exp_base: u32,
    max_quote: I80F48,
    shape_num: I80F48,
    shape_denom: I80F48,
    spread: I80F48,
    symbol: String,
    maker_symbol: String,
    maker_identifier: Pubkey,
    maker_is_derivative: bool,
    hedger_symbol: String,
    hedger_identifier: Pubkey,
    hedger_is_derivative: bool,
}

impl ShapeFunctionInventoryManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        maker_symbol: String,
        maker_identifier: Pubkey,
        maker_is_derivative: bool,
        hedger_symbol: String,
        hedger_identifier: Pubkey,
        hedger_is_derivative: bool,
        decimals: u8,
        exp_base: u32,
        max_quote: I80F48,
        shape_num: I80F48,
        shape_denom: I80F48,
        spread: I80F48,
    ) -> Self {
        let symbol = format!("{}/{}", maker_symbol, hedger_symbol);
        Self {
            maker_symbol,
            maker_identifier: maker_identifier,
            maker_is_derivative: maker_is_derivative,
            hedger_symbol,
            hedger_identifier: hedger_identifier,
            hedger_is_derivative: hedger_is_derivative,
            decimals,
            exp_base,
            max_quote,
            shape_num,
            shape_denom,
            spread,
            symbol,
        }
    }
}

impl Identifier for ShapeFunctionInventoryManager {
    fn symbol(&self) -> &str {
        &self.symbol
    }
}

impl InventoryManager for ShapeFunctionInventoryManager
where
    ShapeFunctionInventoryManager: Identifier,
{
    type Input = GlobalContext;

    fn get_delta(&self, ctx: &GlobalContext) -> I80F48 {
        let user_ctx = &ctx.user;

        let maker_sub_account_ctx = user_ctx.get_sub_account_with_position(&self.maker_identifier);
        let maker_position = if let Some(maker_sub_account_ctx) = maker_sub_account_ctx {
            match maker_sub_account_ctx.get_position(&self.maker_identifier) {
                Some(pos) => {
                    if self.maker_is_derivative {
                        pos.derivative.total_position()
                    } else {
                        pos.spot.position()
                    }
                }
                None => I80F48::ZERO,
            }
        } else {
            I80F48::ZERO
        };

        let hedger_sub_account_ctx =
            user_ctx.get_sub_account_with_position(&self.hedger_identifier);
        let hedger_position = if let Some(hedger_sub_account_ctx) = hedger_sub_account_ctx {
            match hedger_sub_account_ctx.get_position(&self.hedger_identifier) {
                Some(pos) => {
                    if self.hedger_is_derivative {
                        pos.derivative.total_position()
                    } else {
                        pos.spot.position()
                    }
                }
                None => I80F48::ZERO,
            }
        } else {
            I80F48::ZERO
        };

        let delta = maker_position - hedger_position;
        info!(
            "[{}] Maker Position: {} - Hedger Position: {} - Delta: {}",
            self.symbol(),
            maker_position,
            hedger_position,
            delta
        );

        delta
    }

    fn get_quote_size(&self, absolute_delta: I80F48) -> I80F48 {
        let shaped_delta = self.shape_num.checked_mul(absolute_delta).unwrap();
        let divided_shaped_delta = shaped_delta
            .checked_div(self.shape_denom)
            .unwrap()
            .to_num::<u32>();
        let divisor = I80F48::from(self.exp_base.pow(divided_shaped_delta));
        self.max_quote.checked_div(divisor).unwrap()
    }

    fn get_quote_volumes(&self, ctx: &GlobalContext) -> QuoteVolumes {
        let current_delta = self.get_delta(ctx);

        let adjusted_vol = self.get_quote_size(current_delta.abs());
        let (bid_size, ask_size) = if current_delta < I80F48::ZERO {
            (self.max_quote, adjusted_vol)
        } else {
            (adjusted_vol, self.max_quote)
        };
        let res = QuoteVolumes {
            delta: current_delta,
            bid_size,
            ask_size,
        };
        info!("[{}] Quote Volumes: {:?}", self.symbol(), res);
        res
    }

    fn get_spread(&self, oracle_price: I80F48) -> SpreadInfo {
        let ask = oracle_price.checked_mul(self.spread).unwrap();
        let bid = oracle_price.checked_div(self.spread).unwrap();

        let res = SpreadInfo {
            oracle_price,
            bid,
            ask,
        };
        info!("[{}] Quote Volumes: {:?}", self.symbol(), res);
        res
    }
}
