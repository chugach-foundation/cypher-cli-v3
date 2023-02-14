use anchor_lang::prelude::Pubkey;
use cypher_client::{MarginCollateralRatioType, Side, SubAccountMargining};
use cypher_utils::contexts::{CacheContext, UserContext};
use log::info;

use crate::common::info::ClearingInfo;

#[derive(Default)]
pub struct OpenOrder {
    pub order_id: u128,
    pub side: Side,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LiquidationType {
    None,
    Account { account: Pubkey },
    SubAccount { sub_account: Pubkey },
}

impl Default for LiquidationType {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PositionType {
    Spot,
    Derivative,
}

impl Default for PositionType {
    fn default() -> Self {
        Self::Spot
    }
}

#[derive(Debug, Default, Clone)]
pub struct PositionInfo {
    pub position_type: PositionType,
    pub identifier: Pubkey,
    pub account: Pubkey,
    pub sub_account: Pubkey,
}

#[derive(Default)]
pub struct LiquidationCheck {
    pub liquidation_type: LiquidationType,
    pub positions_info: Vec<PositionInfo>,
}

pub fn check_collateral(
    cache_ctx: &CacheContext,
    liqee_ctx: &UserContext,
    liqee_clearing: &ClearingInfo,
) -> LiquidationCheck {
    let account_ctx = &liqee_ctx.account_ctx;

    info!(
        "Checking if account {} is eligible for liquidation..",
        account_ctx.address
    );

    // let's check if the account is eligible for liquidation
    let margin_maint_ratio = liqee_clearing.state.maint_margin_ratio();
    let margin_c_ratio = account_ctx.state.get_margin_c_ratio();

    info!(
        "Margin Maint Ratio: {} - Margin C Ratio: {}",
        margin_maint_ratio, margin_c_ratio
    );

    if margin_c_ratio < margin_maint_ratio {
        info!(
            "Liqee: {} - Account: {} - Margin C Ratio below Maintenance. C Ratio: {}",
            account_ctx.state.authority, account_ctx.address, margin_c_ratio
        );

        let mut positions_info = Vec::new();

        for sub_account_ctx in liqee_ctx.sub_account_ctxs.iter() {
            // if the account is cross margined we need to check sub accounts to see if we can improve the c-ratio before attempting to liquidate the account
            if sub_account_ctx.state.margining_type == SubAccountMargining::Cross {
                // let's check if the user has unsettled funds or open orders which would increase the c-ratio
                for position in sub_account_ctx.state.positions.iter() {
                    if position.spot.token_mint != Pubkey::default() {
                        if position.spot.open_orders_cache.coin_total != 0
                            || position.spot.open_orders_cache.pc_total != 0
                        {
                            positions_info.push(PositionInfo {
                                position_type: PositionType::Spot,
                                identifier: position.spot.token_mint,
                                account: account_ctx.address,
                                sub_account: sub_account_ctx.address,
                            });
                        }
                    }

                    if position.derivative.market != Pubkey::default() {
                        if position.derivative.open_orders_cache.coin_total != 0
                            || position.derivative.open_orders_cache.pc_total != 0
                        {
                            positions_info.push(PositionInfo {
                                position_type: PositionType::Derivative,
                                identifier: position.derivative.market,
                                account: account_ctx.address,
                                sub_account: sub_account_ctx.address,
                            });
                        }
                    }
                }
            }
        }

        return LiquidationCheck {
            liquidation_type: LiquidationType::Account {
                account: account_ctx.address,
            },
            positions_info,
        };
    }

    // let's check if isolated sub accounts are eligible for liquidation
    for sub_account_ctx in liqee_ctx.sub_account_ctxs.iter() {
        if sub_account_ctx.state.margining_type == SubAccountMargining::Isolated {
            info!(
                "Checking if sub account {} is eligible for liquidation..",
                sub_account_ctx.state.authority
            );
            let margin_c_ratio = sub_account_ctx.state.get_margin_c_ratio(
                &cache_ctx.state.as_ref(),
                MarginCollateralRatioType::Maintenance,
            );
            if margin_c_ratio < margin_maint_ratio {
                info!(
                    "Liqee: {} - Sub Account: {} - Margin C Ratio below Maintenance. C Ratio: {}",
                    account_ctx.state.authority, sub_account_ctx.address, margin_c_ratio
                );
                let mut positions_info = Vec::new();

                // let's check if the user has unsettled funds or open orders which would increase the c-ratio
                for position in sub_account_ctx.state.positions.iter() {
                    if position.spot.token_mint != Pubkey::default() {
                        if position.spot.open_orders_cache.coin_total != 0
                            || position.spot.open_orders_cache.pc_total != 0
                        {
                            positions_info.push(PositionInfo {
                                position_type: PositionType::Spot,
                                identifier: position.spot.token_mint,
                                account: account_ctx.address,
                                sub_account: sub_account_ctx.address,
                            });
                        }
                    }

                    if position.derivative.market != Pubkey::default() {
                        if position.derivative.open_orders_cache.coin_total != 0
                            || position.derivative.open_orders_cache.pc_total != 0
                        {
                            positions_info.push(PositionInfo {
                                position_type: PositionType::Derivative,
                                identifier: position.derivative.market,
                                account: account_ctx.address,
                                sub_account: sub_account_ctx.address,
                            });
                        }
                    }
                }

                return LiquidationCheck {
                    liquidation_type: LiquidationType::SubAccount {
                        sub_account: sub_account_ctx.address,
                    },
                    positions_info,
                    ..Default::default()
                };
            }
        }
    }

    return LiquidationCheck {
        liquidation_type: LiquidationType::None,
        ..Default::default()
    };
}

// todo: there's many things that can be changed here
// such as using an isolated sub account to perform liquidations
pub fn check_can_liquidate(
    cache_ctx: &CacheContext,
    liqor_ctx: &UserContext,
    liqor_clearing: &ClearingInfo,
) -> bool {
    let account_ctx = &liqor_ctx.account_ctx;

    info!(
        "Checking if liquidator {} can perform liquidation..",
        account_ctx.address
    );

    // let's check if the liquidator can still perform liquidations
    let init_margin_ratio = liqor_clearing.state.init_margin_ratio();
    let margin_c_ratio = account_ctx.state.get_margin_c_ratio();

    if margin_c_ratio >= init_margin_ratio {
        return true;
    }

    false
}
