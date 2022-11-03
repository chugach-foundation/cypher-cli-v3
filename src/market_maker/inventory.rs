use {
    cypher_client::CypherSubAccount,
    fixed::types::I80F48,
    log::info,
    serde::{Deserialize, Serialize},
    std::sync::Arc,
};

use super::constants::BPS_UNIT;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryManagerConfig {
    pub initial_capital: u64,
    pub exp_base: i64,
    pub max_quote: i64,
    pub shape_num: u32,
    pub shape_denom: u32,
    pub spread: u8,
}

pub struct InventoryManager {
    decimals: u8,
    exp_base: i64,
    max_quote: i64,
    shape_num: u32,
    shape_denom: u32,
    spread: u8,
}

#[derive(Debug, Default)]
pub struct QuoteVolumes {
    pub delta: i64,
    pub bid_size: i128,
    pub ask_size: i128,
}

impl InventoryManager {
    pub fn default() -> Self {
        Self {
            decimals: u8::default(),
            exp_base: i64::default(),
            max_quote: i64::default(),
            shape_num: u32::default(),
            shape_denom: u32::default(),
            spread: u8::default(),
        }
    }

    pub fn new(
        decimals: u8,
        exp_base: i64,
        max_quote: i64,
        shape_num: u32,
        shape_denom: u32,
        spread: u8,
    ) -> Self {
        Self {
            decimals,
            exp_base,
            max_quote,
            shape_num,
            shape_denom,
            spread,
        }
    }

    // #[inline(always)]
    // pub async fn get_quote_volumes(
    //     &self,
    //     user: &CypherUser,
    // ) -> QuoteVolumes {
    //     let current_delta = self.get_user_delta(sub_account);

    //     let adjusted_vol = self.adj_quote_size(current_delta.abs().try_into().unwrap());
    //     let (bid_size, ask_size) = if current_delta < 0 {
    //         (self.max_quote as i128, adjusted_vol)
    //     } else {
    //         (adjusted_vol, self.max_quote as i128)
    //     };
    //     QuoteVolumes {
    //         delta: current_delta,
    //         bid_size,
    //         ask_size,
    //     }
    // }

    // #[inline(always)]
    // fn get_user_delta(
    //     &self,
    //     sub_account: &CypherSubAccount,
    // ) -> i64 {
    //     let maybe_pos = sub_account.get(self.market_idx);

    //     let user_pos = match maybe_pos {
    //         Some(position) => position,
    //         None => {
    //             return 0;
    //         }
    //     };

    //     info!(
    //         "[INVMGR] Position: {}.",
    //         user_pos.base_borrows(),
    //     );
    //     let quote_divisor = I80F48::from_num::<u64>(10_u64.checked_pow(6).unwrap());
    //     let token_divisor = I80F48::from_num::<u64>(10_u64.checked_pow(self.decimals as u32).unwrap());

    //     let long_pos = user_pos.base_deposits().as_u64(0) as i64 / c_asset_divisor as i64;
    //     let delta = long_pos as i64 - short_pos as i64;

    //     info!(
    //         "[INVMGR] Open Orders Coin Free: {}. Open Orders Coin Total: {}.",
    //         user_pos.oo_info.coin_free, user_pos.oo_info.coin_total,
    //     );

    //     info!(
    //         "[INVMGR] Open Orders Price Coin Free: {}. Open Orders Price Coin Total: {}.",
    //         user_pos.oo_info.pc_free, user_pos.oo_info.pc_total,
    //     );

    //     let assets_val = sub_account.get_assets_value();
    //     let assets_val_ui = assets_val / quote_divisor;
    //     let liabs_val = sub_account.get_liabilities_value();
    //     let liabs_val_ui = liabs_val / quote_divisor;

    //     info!(
    //         "[INVMGR] Assets value: {} - Liabilities value: {} ",
    //         assets_val_ui, liabs_val_ui
    //     );

    //     delta
    // }

    #[inline(always)]
    fn adj_quote_size(&self, abs_delta: u32) -> i128 {
        let shaped_delta = self.shape_num * abs_delta;
        let divided_shaped_delta = shaped_delta / self.shape_denom;
        let divisor: i128 = self.exp_base.pow(divided_shaped_delta).into();
        self.max_quote as i128 / divisor
    }

    #[inline(always)]
    pub fn get_spread(&self, oracle_price: u64) -> (u64, u64) {
        let num = (BPS_UNIT + self.spread as u64) as f64 / BPS_UNIT as f64;
        let best_ask = oracle_price as f64 * num;
        let best_bid = oracle_price as f64 / num;

        (best_bid as u64, best_ask as u64)
    }
}
