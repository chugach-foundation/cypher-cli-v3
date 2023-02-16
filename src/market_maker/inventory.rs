#![allow(dead_code)]
use {fixed::types::I80F48, log::info, solana_sdk::pubkey::Pubkey};

use crate::common::{
    context::GlobalContext,
    inventory::{InventoryManager, QuoteVolumes, SpreadInfo},
};

pub struct ShapeFunctionInventoryManager {
    decimals: u8,
    exp_base: u32,
    max_quote: I80F48,
    shape_num: I80F48,
    shape_denom: I80F48,
    spread: I80F48,
    market_identifier: Pubkey,
    is_derivative: bool,
}

impl ShapeFunctionInventoryManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        market_identifier: Pubkey,
        is_derivative: bool,
        decimals: u8,
        exp_base: u32,
        max_quote: I80F48,
        shape_num: I80F48,
        shape_denom: I80F48,
        spread: I80F48,
    ) -> Self {
        Self {
            market_identifier,
            is_derivative,
            decimals,
            exp_base,
            max_quote,
            shape_num,
            shape_denom,
            spread,
        }
    }
}

impl InventoryManager for ShapeFunctionInventoryManager {
    type Input = GlobalContext;

    fn get_delta(&self, ctx: &GlobalContext) -> I80F48 {
        let user_ctx = &ctx.user;
        let sub_account_ctx = user_ctx.sub_account_ctxs.first(); // might need rework
        let sub_account = match sub_account_ctx {
            Some(a) => a,
            None => return I80F48::ZERO,
        };

        let position = sub_account.get_position(&self.market_identifier);
        let delta = match position {
            Some(pos) => {
                if self.is_derivative {
                    pos.derivative.total_position()
                } else {
                    pos.spot.position()
                }
            }
            None => I80F48::ZERO,
        };

        info!("Current delta: {}", delta);

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
        QuoteVolumes {
            delta: current_delta,
            bid_size,
            ask_size,
        }
    }

    fn get_spread(&self, oracle_price: I80F48) -> SpreadInfo {
        let ask = oracle_price.checked_mul(self.spread).unwrap();
        let bid = oracle_price.checked_div(self.spread).unwrap();

        SpreadInfo {
            oracle_price,
            bid,
            ask,
        }
    }
}
