use crate::common::info::ClearingInfo;
use anchor_lang::prelude::Pubkey;
use cypher_client::{constants::QUOTE_TOKEN_DECIMALS, quote_mint, utils::adjust_decimals};
use cypher_utils::contexts::{CacheContext, UserContext};
use fixed::types::I80F48;
use log::info;

use super::error::Error;

#[derive(Default, Clone)]
pub struct SimulationContext {
    pub cache: CacheContext,
    pub liqor: UserContext,
    pub liqor_clearing: ClearingInfo,
    pub liqor_sub_account_idx: usize,
    pub liqee: UserContext,
    pub liqee_clearing: ClearingInfo,
    pub liqee_sub_account_idx: usize,
}

#[derive(Debug, Default, Clone)]
pub struct SimulateLiquidationArgs {
    pub assets_value: I80F48,
    pub liabilities_value: I80F48,
    pub asset: Pubkey,
    pub liability: Pubkey,
    pub asset_cache_idx: usize,
    pub liability_cache_idx: usize,
    pub asset_decimals: u8,
    pub liability_decimals: u8,
    pub asset_price: I80F48,
    pub liability_price: I80F48,
    pub insurance_fund: I80F48,
}

#[derive(Clone)]
pub enum SimulationType {
    Futures {
        simulation_ctx: SimulationContext,
        simulation_args: SimulateLiquidationArgs,
    },
    Perpetuals {
        simulation_ctx: SimulationContext,
        simulation_args: SimulateLiquidationArgs,
    },
    Spot {
        simulation_ctx: SimulationContext,
        simulation_args: SimulateLiquidationArgs,
    },
}

impl Default for SimulationType {
    fn default() -> Self {
        Self::Spot {
            simulation_ctx: SimulationContext::default(),
            simulation_args: SimulateLiquidationArgs::default(),
        }
    }
}

#[derive(Default, Clone)]
pub struct SimulationResult {
    pub repay_amount: I80F48,
    pub liqee_asset_debit: I80F48,
    pub global_insurance_credit: I80F48,
    pub global_insurance_debit: I80F48,
    pub simulation_type: SimulationType,
}

impl std::fmt::Debug for SimulationResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimulationResult")
            .field("repay_amount", &format!("{:?}", self.repay_amount))
            .field(
                "liqee_asset_debit",
                &format!("{:?}", self.liqee_asset_debit),
            )
            .field(
                "global_insurance_credit",
                &format!("{:?}", self.global_insurance_credit),
            )
            .field(
                "global_insurance_debit",
                &format!("{:?}", self.global_insurance_debit),
            )
            .finish()
    }
}

/// Simulates a liquidation of a given type.
pub fn simulate_liquidation(simulation_type: SimulationType) -> Result<SimulationResult, Error> {
    match simulation_type {
        SimulationType::Futures {
            simulation_ctx,
            simulation_args,
        } => simulate_futures_liquidation(&simulation_ctx, &simulation_args),
        SimulationType::Perpetuals {
            simulation_ctx,
            simulation_args,
        } => simulate_perp_liquidation(&simulation_ctx, &simulation_args),
        SimulationType::Spot {
            simulation_ctx,
            simulation_args,
        } => simulate_spot_liquidation(&simulation_ctx, &simulation_args),
    }
}

/// Simulates a liquidation of tokens.
pub fn simulate_spot_liquidation(
    ctx: &SimulationContext,
    args: &SimulateLiquidationArgs,
) -> Result<SimulationResult, Error> {
    let cache_account = ctx.cache.state.as_ref();
    let liqee_clearing = ctx.liqee_clearing.state.as_ref();
    let liqor_clearing = ctx.liqor_clearing.state.as_ref();
    let liqee_sub_account = ctx.liqee.sub_account_ctxs[ctx.liqee_sub_account_idx]
        .state
        .as_ref();
    let liqor_sub_account = ctx.liqor.sub_account_ctxs[ctx.liqor_sub_account_idx]
        .state
        .as_ref();

    let target_ratio = liqee_clearing.target_margin_ratio();
    let liqor_fee = liqor_clearing.liq_liqor_fee();
    let insurance_fee = liqee_clearing.liq_insurance_fee();

    let asset_cache = cache_account.get_price_cache(args.asset_cache_idx);
    let liability_cache = cache_account.get_price_cache(args.liability_cache_idx);

    // calculate excess liabilities value
    let excess_liabs_value = args
        .liabilities_value
        .checked_mul(target_ratio)
        .and_then(|n| n.checked_sub(args.assets_value))
        .and_then(|n| {
            n.checked_mul(
                target_ratio
                    .checked_sub(liqor_fee)
                    .and_then(|n| n.checked_sub(insurance_fee))
                    .unwrap(),
            )
        })
        .unwrap();
    info!("Excess Liabs Value: {}", excess_liabs_value);

    // calculate loan value in the liability position
    let liqee_liability_position_idx = liqee_sub_account
        .get_position_idx(&args.liability, true)
        .unwrap();
    let loan_value_in_position = adjust_decimals(
        liqee_sub_account
            .get_spot_position(liqee_liability_position_idx)
            .total_position(liability_cache)
            .abs(),
        args.liability_decimals,
    )
    .checked_mul(args.liability_price)
    .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
    .unwrap();
    info!("Loan Value In Position: {}", loan_value_in_position);
    // max repay value is the minimum between excess liabilities and loan value in position
    let max_repay_value = loan_value_in_position.min(excess_liabs_value);

    // check if liqee is bankrupt
    let is_bankrupt = liqee_sub_account.is_bankrupt(liqee_clearing, cache_account)?;
    if is_bankrupt {
        assert_eq!(args.asset, quote_mint::ID);
        // if the liability is the quote token, then we don't need to calculate anything else
        // the final repay amount should be the minimum between the max repay value and the available insurance fund
        // and this repay amount should also be debited from the market's insurance fund
        if args.liability == quote_mint::ID {
            let repay_amount = max_repay_value.min(args.insurance_fund);
            info!(
                "Account {} - Sub account: {} - Bankrupt with quote token liabilities!",
                liqee_sub_account.master_account,
                ctx.liqee.sub_account_ctxs[ctx.liqee_sub_account_idx].address
            );
            let res = SimulationResult {
                repay_amount,
                liqee_asset_debit: I80F48::ZERO,
                global_insurance_credit: I80F48::ZERO,
                global_insurance_debit: repay_amount,
                simulation_type: SimulationType::Spot {
                    simulation_ctx: ctx.clone(),
                    simulation_args: args.clone(),
                },
            };
            return Ok(res);
        }
    }

    // calculate the maximum possible value to swap, based on the liqee's asset market's/token's position
    let max_value_for_swap = if is_bankrupt {
        args.insurance_fund.checked_div(liqor_fee).unwrap()
    } else {
        // get liqee's asset position
        let liqee_asset_position_value = {
            // get the quote token position
            let liqee_asset_position_idx = liqee_sub_account
                .get_position_idx(&args.asset, true)
                .unwrap();
            let liqee_asset_position =
                liqee_sub_account.get_spot_position(liqee_asset_position_idx);
            // calculate liqee's asset position value based on asset price
            adjust_decimals(
                liqee_asset_position.total_position(asset_cache).abs(),
                args.asset_decimals,
            )
            .checked_mul(args.asset_price)
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
            .unwrap()
        };
        // the maximum value for swap is going to be the liqee's asset position value without liqor and insurance fees
        liqee_asset_position_value
            .checked_div(liqor_fee.checked_add(insurance_fee).unwrap())
            .unwrap()
    };
    info!("Max Value For Swap: {}", max_value_for_swap);

    // calculate liquidator's available repayment value based on their liability market's/token's position
    let liqor_repay_position_value = {
        // get the quote token position
        let liqor_liability_position_idx = liqor_sub_account
            .get_position_idx(&args.liability, true)
            .unwrap();
        let liqor_liability_position =
            liqor_sub_account.get_spot_position(liqor_liability_position_idx);
        adjust_decimals(
            liqor_liability_position
                .total_position(liability_cache)
                .abs(),
            args.liability_decimals,
        )
        .checked_mul(args.liability_price)
        .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
        .unwrap()
    };
    info!("Liqor Repay Position Value: {}", liqor_repay_position_value);

    let max_liability_swap_value = max_value_for_swap.min(liqor_repay_position_value);
    info!("Max Liability Swap Value: {}", max_liability_swap_value);
    let repay_value = max_liability_swap_value.min(max_repay_value);
    info!("Repay Value: {}", repay_value);
    let repay_amount = repay_value
        .checked_div(
            args.liability_price
                .checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32)))
                .unwrap(),
        )
        .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(args.liability_decimals as u32))))
        .unwrap();
    info!("Repay Amount: {}", repay_amount);
    let liqor_credit_value = repay_value.checked_mul(liqor_fee).unwrap();
    info!("Liqor Credit Value: {}", liqor_credit_value);

    let (liqee_asset_debit, global_insurance_debit, global_insurance_credit) = if is_bankrupt {
        // this should never be reached
        if args.liability == quote_mint::ID {
            unreachable!()
        } else {
            info!(
                "Account {} - Sub account {} - Bankrupt!",
                liqee_sub_account.master_account,
                ctx.liqee.sub_account_ctxs[ctx.liqee_sub_account_idx].address
            );
            (I80F48::ZERO, liqor_credit_value, I80F48::ZERO)
        }
    } else {
        let global_insurance_credit_value = repay_value.checked_mul(insurance_fee).unwrap();
        let liqee_debit_value = liqor_credit_value
            .checked_add(global_insurance_credit_value)
            .unwrap();
        info!("Liqee Debit Value: {}", liqee_debit_value);
        let liqee_asset_debit = liqee_debit_value
            .checked_div(
                args.asset_price
                    .checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32)))
                    .unwrap(),
            )
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(args.asset_decimals as u32))))
            .unwrap();
        (
            liqee_asset_debit,
            I80F48::ZERO,
            global_insurance_credit_value,
        )
    };

    let res = SimulationResult {
        repay_amount,
        liqee_asset_debit,
        global_insurance_credit,
        global_insurance_debit,
        simulation_type: SimulationType::Spot {
            simulation_ctx: ctx.clone(),
            simulation_args: args.clone(),
        },
    };
    info!("Simulation Result: {:?}", res);

    Ok(res)
}

/// Simulates a liquidation of positions from [`MarketType::PerpetualFuture`]s.
pub fn simulate_perp_liquidation(
    ctx: &SimulationContext,
    args: &SimulateLiquidationArgs,
) -> Result<SimulationResult, Error> {
    let cache_account = ctx.cache.state.as_ref();
    let liqee_clearing = ctx.liqee_clearing.state.as_ref();
    let liqor_clearing = ctx.liqor_clearing.state.as_ref();
    let liqee_sub_account = ctx.liqee.sub_account_ctxs[ctx.liqee_sub_account_idx]
        .state
        .as_ref();
    let liqor_sub_account = ctx.liqor.sub_account_ctxs[ctx.liqor_sub_account_idx]
        .state
        .as_ref();

    let target_ratio = liqee_clearing.target_margin_ratio();
    let liqor_fee = liqor_clearing.liq_liqor_fee();
    let insurance_fee = liqee_clearing.liq_insurance_fee();

    let asset_cache = cache_account.get_price_cache(args.asset_cache_idx);
    let liability_cache = cache_account.get_price_cache(args.liability_cache_idx);

    let (loan_value_in_position, liqee_asset_position_value, liqor_repay_position_value) =
        if args.asset == quote_mint::id() {
            // at this point we know that the liability is a derivative and the asset we're taking
            // from liqee is the quote token
            // calculate loan value in the liability position
            let liqee_liability_position_idx = liqee_sub_account
                .get_position_idx(&args.liability, false)
                .unwrap();
            // calculate liqee's loan value based on the liability price
            let loan_value_in_position = adjust_decimals(
                liqee_sub_account
                    .get_derivative_position(liqee_liability_position_idx)
                    .total_position()
                    .abs(),
                args.liability_decimals,
            )
            .checked_mul(args.liability_price)
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
            .unwrap();

            // get liqee's asset position
            // get the quote token position
            let liqee_asset_position_idx = liqee_sub_account
                .get_position_idx(&quote_mint::ID, true)
                .unwrap();
            let liqee_asset_position =
                liqee_sub_account.get_spot_position(liqee_asset_position_idx);
            // calculate liqee's asset position value based on asset price
            let liqee_asset_position_value = adjust_decimals(
                liqee_asset_position.total_position(asset_cache).abs(),
                args.asset_decimals,
            )
            .checked_mul(args.asset_price)
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
            .unwrap();

            // calculate liquidator's available repayment value based on their liability market's position
            // get the derivative position associated with the given liability
            let liqor_liability_position_idx = liqor_sub_account
                .get_position_idx(&args.liability, false)
                .unwrap();
            let liqor_liability_position =
                liqor_sub_account.get_derivative_position(liqor_liability_position_idx);
            // calculate liqor's repayment position value based on liability price
            let liqor_repay_position_value = adjust_decimals(
                liqor_liability_position.total_position().abs(),
                args.liability_decimals,
            )
            .checked_mul(args.liability_price)
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
            .unwrap();
            (
                loan_value_in_position,
                liqee_asset_position_value,
                liqor_repay_position_value,
            )
        } else if args.liability == quote_mint::id() {
            // at this point we know that the liability is the quote token and the asset
            // we're taking is a derivative position

            // calculate loan value in the liability position
            let liqee_liability_position_idx = liqee_sub_account
                .get_position_idx(&args.liability, true)
                .unwrap();
            // calculate liqee's loan value based on the liability price
            let loan_value_in_position = adjust_decimals(
                liqee_sub_account
                    .get_spot_position(liqee_liability_position_idx)
                    .total_position(liability_cache)
                    .abs(),
                args.liability_decimals,
            )
            .checked_mul(args.liability_price)
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
            .unwrap();

            // get liqee's asset position
            // get the derivative position associated with the given asset
            let liqee_asset_position_idx = liqee_sub_account
                .get_position_idx(&args.asset, false)
                .unwrap();
            let liqee_asset_position =
                liqee_sub_account.get_derivative_position(liqee_asset_position_idx);
            // calculate liqee's asset position value based on asset price
            let liqee_asset_position_value = adjust_decimals(
                liqee_asset_position.total_position().abs(),
                args.asset_decimals,
            )
            .checked_mul(args.asset_price)
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
            .unwrap();

            // calculate liquidator's available repayment value based on their liability market's/token's position
            // get the quote token position
            let liqor_liability_position_idx = liqor_sub_account
                .get_position_idx(&quote_mint::ID, true)
                .unwrap();
            let liqor_liability_position =
                liqor_sub_account.get_spot_position(liqor_liability_position_idx);
            // calculate liqor's repayment position value based on liability price
            let liqor_repay_position_value = adjust_decimals(
                liqor_liability_position
                    .total_position(liability_cache)
                    .abs(),
                args.liability_decimals,
            )
            .checked_mul(args.liability_price)
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
            .unwrap();
            (
                loan_value_in_position,
                liqee_asset_position_value,
                liqor_repay_position_value,
            )
        } else {
            // at this point we know that the liability is a derivative position
            // and we're taking from liqee another derivative position

            // calculate loan value in the liability position
            let liqee_liability_position_idx = liqee_sub_account
                .get_position_idx(&args.liability, false)
                .unwrap();
            // calculate liqee's loan value based on the liability price
            let loan_value_in_position = adjust_decimals(
                liqee_sub_account
                    .get_derivative_position(liqee_liability_position_idx)
                    .total_position()
                    .abs(),
                args.liability_decimals,
            )
            .checked_mul(args.liability_price)
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
            .unwrap();

            // get liqee's asset position
            // get the derivative position associated with the given asset
            let liqee_asset_position_idx = liqee_sub_account
                .get_position_idx(&args.asset, false)
                .unwrap();
            let liqee_asset_position =
                liqee_sub_account.get_derivative_position(liqee_asset_position_idx);
            // calculate liqee's asset position value based on asset price
            let liqee_asset_position_value = adjust_decimals(
                liqee_asset_position.total_position().abs(),
                args.asset_decimals,
            )
            .checked_mul(args.asset_price)
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
            .unwrap();

            // calculate liquidator's available repayment value based on their liability market's/token's position
            // get the derivative position associated with the given liability
            let liqor_liability_position_idx = liqor_sub_account
                .get_position_idx(&args.liability, false)
                .unwrap();
            let liqor_liability_position =
                liqor_sub_account.get_derivative_position(liqor_liability_position_idx);
            // calculate liqor's repayment position value based on liability price
            let liqor_repay_position_value = adjust_decimals(
                liqor_liability_position.total_position().abs(),
                args.liability_decimals,
            )
            .checked_mul(args.liability_price)
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
            .unwrap();
            (
                loan_value_in_position,
                liqee_asset_position_value,
                liqor_repay_position_value,
            )
        };

    // calculate excess liabilities value
    let excess_liabs_value = args
        .liabilities_value
        .checked_mul(target_ratio)
        .and_then(|n| n.checked_sub(args.assets_value))
        .and_then(|n| {
            n.checked_mul(
                target_ratio
                    .checked_sub(liqor_fee)
                    .and_then(|n| n.checked_sub(insurance_fee))
                    .unwrap(),
            )
        })
        .unwrap();
    info!("Excess Liabs Value: {}", excess_liabs_value);
    info!("Loan Value In Position: {}", loan_value_in_position);
    // max repay value is the minimum between excess liabilities and loan value in position
    let max_repay_value = loan_value_in_position.min(excess_liabs_value);

    // check if liqee is bankrupt
    let is_bankrupt = liqee_sub_account.is_bankrupt(liqee_clearing, cache_account)?;
    if is_bankrupt {
        assert_eq!(args.asset, quote_mint::ID);
        // if the liability is the quote token, then we don't need to calculate anything else
        // the final repay amount should be the minimum between the max repay value and the available insurance fund
        // and this repay amount should also be debited from the market's insurance fund
        if args.liability == quote_mint::ID {
            let repay_amount = max_repay_value.min(args.insurance_fund);
            info!(
                "Account {} - Sub account: {} - Bankrupt with quote token liabilities!",
                liqee_sub_account.master_account,
                ctx.liqee.sub_account_ctxs[ctx.liqee_sub_account_idx].address
            );
            let res = SimulationResult {
                repay_amount,
                liqee_asset_debit: I80F48::ZERO,
                global_insurance_credit: I80F48::ZERO,
                global_insurance_debit: repay_amount,
                simulation_type: SimulationType::Spot {
                    simulation_ctx: ctx.clone(),
                    simulation_args: args.clone(),
                },
            };
            return Ok(res);
        }
    }

    // calculate the maximum possible value to swap, based on the liqee's asset market's/token's position
    let max_value_for_swap = if is_bankrupt {
        args.insurance_fund.checked_div(liqor_fee).unwrap()
    } else {
        // the maximum value for swap is going to be the liqee's asset position value without liqor and insurance fees
        liqee_asset_position_value
            .checked_div(liqor_fee.checked_add(insurance_fee).unwrap())
            .unwrap()
    };
    info!("Max Value For Swap: {}", max_value_for_swap);

    let max_liability_swap_value = max_value_for_swap.min(liqor_repay_position_value);
    info!("Max Liability Swap Value: {}", max_liability_swap_value);
    let repay_value = max_liability_swap_value.min(max_repay_value);
    info!("Repay Value: {}", repay_value);
    let repay_amount = repay_value
        .checked_div(
            args.liability_price
                .checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32)))
                .unwrap(),
        )
        .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(args.liability_decimals as u32))))
        .unwrap();
    info!("Repay Amount: {}", repay_amount);
    let liqor_credit_value = repay_value.checked_mul(liqor_fee).unwrap();
    info!("Liqor Credit Value: {}", liqor_credit_value);

    let (liqee_asset_debit, global_insurance_debit, global_insurance_credit) = if is_bankrupt {
        // this should never be reached
        if args.liability == quote_mint::ID {
            unreachable!()
        } else {
            (I80F48::ZERO, liqor_credit_value, I80F48::ZERO)
        }
    } else {
        let global_insurance_credit_value = repay_value.checked_mul(insurance_fee).unwrap();
        let liqee_debit_value = liqor_credit_value
            .checked_add(global_insurance_credit_value)
            .unwrap();
        info!("Liqee Debit Value: {}", liqee_debit_value);
        let liqee_asset_debit = liqee_debit_value
            .checked_div(
                args.asset_price
                    .checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32)))
                    .unwrap(),
            )
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(args.asset_decimals as u32))))
            .unwrap();
        (
            liqee_asset_debit,
            I80F48::ZERO,
            global_insurance_credit_value,
        )
    };

    let res = SimulationResult {
        repay_amount,
        liqee_asset_debit,
        global_insurance_credit,
        global_insurance_debit,
        simulation_type: SimulationType::Spot {
            simulation_ctx: ctx.clone(),
            simulation_args: args.clone(),
        },
    };
    info!("Simulation Result: {:?}", res);

    Ok(res)
}

/// Simulates a liquidation of positions from [`MarketType::IndexFuture`] or [`MarketType::PairFutures`].
pub fn simulate_futures_liquidation(
    ctx: &SimulationContext,
    args: &SimulateLiquidationArgs,
) -> Result<SimulationResult, Error> {
    let cache_account = ctx.cache.state.as_ref();
    let liqee_clearing = ctx.liqee_clearing.state.as_ref();
    let liqor_clearing = ctx.liqor_clearing.state.as_ref();
    let liqee_sub_account = ctx.liqee.sub_account_ctxs[ctx.liqee_sub_account_idx]
        .state
        .as_ref();
    let liqor_sub_account = ctx.liqor.sub_account_ctxs[ctx.liqor_sub_account_idx]
        .state
        .as_ref();

    let target_ratio = liqee_clearing.target_margin_ratio();
    let liqor_fee = liqor_clearing.liq_liqor_fee();
    let insurance_fee = liqee_clearing.liq_insurance_fee();

    let asset_cache = cache_account.get_price_cache(args.asset_cache_idx);
    let liability_cache = cache_account.get_price_cache(args.liability_cache_idx);

    let (loan_value_in_position, liqee_asset_position_value, liqor_repay_position_value) =
        if args.asset == quote_mint::id() {
            // at this point we know that the liability is a derivative and the asset we're taking
            // from liqee is the quote token

            // calculate loan value in the liability position
            let liqee_liability_position_idx = liqee_sub_account
                .get_position_idx(&args.liability, false)
                .unwrap();
            // calculate liqee's loan value based on the liability price
            let loan_value_in_position = adjust_decimals(
                liqee_sub_account
                    .get_derivative_position(liqee_liability_position_idx)
                    .total_position()
                    .abs(),
                args.liability_decimals,
            )
            .checked_mul(args.liability_price)
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
            .unwrap();

            // get liqee's asset position
            // get the quote token position
            let liqee_asset_position_idx = liqee_sub_account
                .get_position_idx(&quote_mint::ID, true)
                .unwrap();
            let liqee_asset_position =
                liqee_sub_account.get_spot_position(liqee_asset_position_idx);
            // calculate liqee's asset position value based on asset price
            let liqee_asset_position_value = adjust_decimals(
                liqee_asset_position.total_position(asset_cache).abs(),
                args.asset_decimals,
            )
            .checked_mul(args.asset_price)
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
            .unwrap();

            // calculate liquidator's available repayment value based on their liability market's position
            // get the derivative position associated with the given liability
            let liqor_liability_position_idx = liqor_sub_account
                .get_position_idx(&args.liability, false)
                .unwrap();
            let liqor_liability_position =
                liqor_sub_account.get_derivative_position(liqor_liability_position_idx);
            // calculate liqor's repayment position value based on liability price
            let liqor_repay_position_value = adjust_decimals(
                liqor_liability_position.base_position().abs(),
                args.liability_decimals,
            )
            .checked_mul(args.liability_price)
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
            .unwrap();
            (
                loan_value_in_position,
                liqee_asset_position_value,
                liqor_repay_position_value,
            )
        } else if args.liability == quote_mint::id() {
            // at this point we know that the liability is the quote token and the asset
            // we're taking is a derivative position

            // calculate loan value in the liability position
            let liqee_liability_position_idx = liqee_sub_account
                .get_position_idx(&args.liability, true)
                .unwrap();
            // calculate liqee's loan value based on the liability price
            let loan_value_in_position = adjust_decimals(
                liqee_sub_account
                    .get_spot_position(liqee_liability_position_idx)
                    .total_position(liability_cache)
                    .abs(),
                args.liability_decimals,
            )
            .checked_mul(args.liability_price)
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
            .unwrap();

            // get liqee's asset position
            // get the derivative position associated with the given asset
            let liqee_asset_position_idx = liqee_sub_account
                .get_position_idx(&args.asset, false)
                .unwrap();
            let liqee_asset_position =
                liqee_sub_account.get_derivative_position(liqee_asset_position_idx);
            // calculate liqee's asset position value based on asset price
            let liqee_asset_position_value = adjust_decimals(
                liqee_asset_position.total_position().abs(),
                args.asset_decimals,
            )
            .checked_mul(args.asset_price)
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
            .unwrap();

            // calculate liquidator's available repayment value based on their liability market's/token's position
            // get the quote token position
            let liqor_liability_position_idx = liqor_sub_account
                .get_position_idx(&quote_mint::ID, true)
                .unwrap();
            let liqor_liability_position =
                liqor_sub_account.get_spot_position(liqor_liability_position_idx);
            // calculate liqor's repayment position value based on liability price
            let liqor_repay_position_value = adjust_decimals(
                liqor_liability_position
                    .total_position(liability_cache)
                    .abs(),
                args.liability_decimals,
            )
            .checked_mul(args.liability_price)
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
            .unwrap();
            (
                loan_value_in_position,
                liqee_asset_position_value,
                liqor_repay_position_value,
            )
        } else {
            // at this point we know that the liability is a derivative position
            // and we're taking from liqee another derivative position

            // calculate loan value in the liability position
            let liqee_liability_position_idx = liqee_sub_account
                .get_position_idx(&args.liability, false)
                .unwrap();
            // calculate liqee's loan value based on the liability price
            let loan_value_in_position = adjust_decimals(
                liqee_sub_account
                    .get_derivative_position(liqee_liability_position_idx)
                    .total_position()
                    .abs(),
                args.liability_decimals,
            )
            .checked_mul(args.liability_price)
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
            .unwrap();

            // get liqee's asset position
            // get the derivative position associated with the given asset
            let liqee_asset_position_idx = liqee_sub_account
                .get_position_idx(&args.asset, false)
                .unwrap();
            let liqee_asset_position =
                liqee_sub_account.get_derivative_position(liqee_asset_position_idx);
            // calculate liqee's asset position value based on asset price
            let liqee_asset_position_value = adjust_decimals(
                liqee_asset_position.total_position().abs(),
                args.asset_decimals,
            )
            .checked_mul(args.asset_price)
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
            .unwrap();

            // calculate liquidator's available repayment value based on their liability market's/token's position
            // get the derivative position associated with the given liability
            let liqor_liability_position_idx = liqor_sub_account
                .get_position_idx(&args.liability, false)
                .unwrap();
            let liqor_liability_position =
                liqor_sub_account.get_derivative_position(liqor_liability_position_idx);
            // calculate liqor's repayment position value based on liability price
            let liqor_repay_position_value = adjust_decimals(
                liqor_liability_position.total_position().abs(),
                args.liability_decimals,
            )
            .checked_mul(args.liability_price)
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32))))
            .unwrap();
            (
                loan_value_in_position,
                liqee_asset_position_value,
                liqor_repay_position_value,
            )
        };

    // calculate excess liabilities value
    let excess_liabs_value = args
        .liabilities_value
        .checked_mul(target_ratio)
        .and_then(|n| n.checked_sub(args.assets_value))
        .and_then(|n| {
            n.checked_mul(
                target_ratio
                    .checked_sub(liqor_fee)
                    .and_then(|n| n.checked_sub(insurance_fee))
                    .unwrap(),
            )
        })
        .unwrap();
    info!("Excess Liabs Value: {}", excess_liabs_value);
    info!("Loan Value In Position: {}", loan_value_in_position);
    // max repay value is the minimum between excess liabilities and loan value in position
    let max_repay_value = loan_value_in_position.min(excess_liabs_value);

    // check if liqee is bankrupt
    let is_bankrupt = liqee_sub_account.is_bankrupt(liqee_clearing, cache_account)?;
    if is_bankrupt {
        assert_eq!(args.asset, quote_mint::ID);
        // if the liability is the quote token, then we don't need to calculate anything else
        // the final repay amount should be the minimum between the max repay value and the available insurance fund
        // and this repay amount should also be debited from the market's insurance fund
        if args.liability == quote_mint::ID {
            let repay_amount = max_repay_value.min(args.insurance_fund);
            info!(
                "Account {} - Sub account: {} - Bankrupt with quote token liabilities!",
                liqee_sub_account.master_account,
                ctx.liqee.sub_account_ctxs[ctx.liqee_sub_account_idx].address
            );
            let res = SimulationResult {
                repay_amount,
                liqee_asset_debit: I80F48::ZERO,
                global_insurance_credit: I80F48::ZERO,
                global_insurance_debit: repay_amount,
                simulation_type: SimulationType::Spot {
                    simulation_ctx: ctx.clone(),
                    simulation_args: args.clone(),
                },
            };
            return Ok(res);
        }
    }

    // calculate the maximum possible value to swap, based on the liqee's asset market's/token's position
    let max_value_for_swap = if is_bankrupt {
        args.insurance_fund.checked_div(liqor_fee).unwrap()
    } else {
        // the maximum value for swap is going to be the liqee's asset position value without liqor and insurance fees
        liqee_asset_position_value
            .checked_div(liqor_fee.checked_add(insurance_fee).unwrap())
            .unwrap()
    };
    info!("Max Value For Swap: {}", max_value_for_swap);

    let max_liability_swap_value = max_value_for_swap.min(liqor_repay_position_value);
    info!("Max Liability Swap Value: {}", max_liability_swap_value);
    let repay_value = max_liability_swap_value.min(max_repay_value);
    info!("Repay Value: {}", repay_value);
    let repay_amount = repay_value
        .checked_div(
            args.liability_price
                .checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32)))
                .unwrap(),
        )
        .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(args.liability_decimals as u32))))
        .unwrap();
    info!("Repay Amount: {}", repay_amount);
    let liqor_credit_value = repay_value.checked_mul(liqor_fee).unwrap();
    info!("Liqor Credit Value: {}", liqor_credit_value);

    let (liqee_asset_debit, global_insurance_debit, global_insurance_credit) = if is_bankrupt {
        // this should never be reached
        if args.liability == quote_mint::ID {
            unreachable!()
        } else {
            (I80F48::ZERO, liqor_credit_value, I80F48::ZERO)
        }
    } else {
        let global_insurance_credit_value = repay_value.checked_mul(insurance_fee).unwrap();
        let liqee_debit_value = liqor_credit_value
            .checked_add(global_insurance_credit_value)
            .unwrap();
        info!("Liqee Debit Value: {}", liqee_debit_value);
        let liqee_asset_debit = liqee_debit_value
            .checked_div(
                args.asset_price
                    .checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32)))
                    .unwrap(),
            )
            .and_then(|n| n.checked_mul(I80F48::from(10u64.pow(args.asset_decimals as u32))))
            .unwrap();
        (
            liqee_asset_debit,
            I80F48::ZERO,
            global_insurance_credit_value,
        )
    };

    let res = SimulationResult {
        repay_amount,
        liqee_asset_debit,
        global_insurance_credit,
        global_insurance_debit,
        simulation_type: SimulationType::Spot {
            simulation_ctx: ctx.clone(),
            simulation_args: args.clone(),
        },
    };
    info!("Simulation Result: {:?}", res);

    Ok(res)
}
