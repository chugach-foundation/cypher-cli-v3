#![allow(clippy::too_many_arguments)]
pub mod accounts;
pub mod check;
pub mod config;
pub mod error;
pub mod simulation;

use anchor_lang::prelude::Pubkey;
use cypher_client::{
    cache_account,
    constants::QUOTE_TOKEN_DECIMALS,
    instructions::{
        liquidate_futures_position as liquidate_futures_position_ix,
        liquidate_perp_position as liquidate_perp_position_ix,
        liquidate_spot_position as liquidate_spot_position_ix,
        update_account_margin as update_account_margin_ix,
    },
    quote_mint,
    utils::{adjust_decimals, derive_pool_address, derive_pool_node_address},
    Clearing, DerivativePosition, MarginCollateralRatioType, MarketType, SpotPosition,
};
use cypher_utils::{
    contexts::{CacheContext, CypherContext, SubAccountContext, UserContext},
    utils::{encode_string, get_cypher_zero_copy_account, send_transactions},
};
use dashmap::DashMap;
use fixed::types::I80F48;
use log::{info, warn};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::instruction::Instruction;

use std::{sync::Arc, time::SystemTime};
use tokio::sync::{broadcast::Sender, RwLock};

use crate::{
    common::{
        context::{builder::ContextBuilder, GlobalContext},
        info::{ClearingInfo, UserInfo},
    },
    liquidator::{
        check::{check_can_liquidate, check_collateral, LiquidationType},
        error::Error,
        simulation::SimulationResult,
    },
};

use self::simulation::{
    simulate_liquidation, SimulateLiquidationArgs, SimulationContext, SimulationType,
};

pub struct Liquidator {
    rpc_client: Arc<RpcClient>,
    shutdown_sender: Arc<Sender<bool>>,
    global_context_builder: Arc<dyn ContextBuilder<Output = GlobalContext>>,
    update_sender: Arc<Sender<UserContext>>,
    user_info: UserInfo,
    clearings_map: DashMap<Pubkey, Box<Clearing>>,
    users_map: DashMap<Pubkey, UserContext>,
    cache_ctx: RwLock<CacheContext>,
    liqor_ctx: RwLock<UserContext>,
    cypher_ctx: CypherContext,
}

impl Liquidator {
    pub fn new(
        rpc_client: Arc<RpcClient>,
        global_context_builder: Arc<dyn ContextBuilder<Output = GlobalContext>>,
        update_sender: Arc<Sender<UserContext>>,
        shutdown_sender: Arc<Sender<bool>>,
        cypher_ctx: CypherContext,
        user_info: UserInfo,
    ) -> Self {
        Self {
            rpc_client,
            global_context_builder,
            update_sender,
            shutdown_sender,
            user_info,
            cypher_ctx,
            clearings_map: DashMap::new(),
            users_map: DashMap::new(),
            cache_ctx: RwLock::new(CacheContext::default()),
            liqor_ctx: RwLock::new(UserContext::default()),
        }
    }

    pub async fn start(&self) {
        let mut g_ctx_update_receiver = self.global_context_builder.subscribe();
        let mut user_update_receiver = self.update_sender.subscribe();
        let mut shutdown_receiver = self.shutdown_sender.subscribe();

        loop {
            tokio::select! {
                g_ctx_update = g_ctx_update_receiver.recv() => {
                    match g_ctx_update {
                        Ok(update) => {
                            *self.cache_ctx.write().await = update.cache;
                            *self.liqor_ctx.write().await = update.user;
                            info!("Successfully processed global context update.");
                            for user_ctx in self.users_map.iter() {
                                let start = SystemTime::now();
                                match self.process(user_ctx.value()).await {
                                    Ok(()) => info!("Successfully processed user context."),
                                    Err(e) => {
                                        warn!("There was an error processing user context. Error: {:?}", e);
                                    }
                                };
                                let delta = start.elapsed().unwrap();
                                info!("Time elapsed: {:?}", delta);
                            }
                        },
                        Err(_) => {
                            warn!("There was an error receiving global context update.");
                            continue;
                        }
                    }
                }
                user_update = user_update_receiver.recv() => {
                    match user_update {
                        Ok(user_ctx) => {
                            self.users_map.insert(user_ctx.authority, user_ctx);
                        },
                        Err(e) => {
                            warn!("There was an error processing user update. Error: {:?}", e);
                        }
                    }

                },
                _ = shutdown_receiver.recv() => {
                    info!("Received shutdown signal, stopping.");
                    break;
                }
            }
        }
    }

    pub async fn process(&self, liqee_ctx: &UserContext) -> Result<(), Error> {
        info!("Running liquidator logic..");

        let cache_ctx = self.cache_ctx.read().await;
        let liqor_ctx = self.liqor_ctx.read().await;
        if liqor_ctx.account_ctx.address == Default::default() {
            return Ok(());
        }
        let liqor_clearing = match self
            .clearings_map
            .get(&liqor_ctx.account_ctx.state.clearing)
        {
            Some(c) => ClearingInfo {
                state: c.clone(),
                address: liqor_ctx.account_ctx.state.clearing,
            },
            None => {
                let clearing = get_cypher_zero_copy_account::<Clearing>(
                    &self.rpc_client,
                    &liqor_ctx.account_ctx.state.clearing,
                )
                .await?;
                self.clearings_map
                    .insert(liqor_ctx.account_ctx.state.clearing, clearing.clone());
                ClearingInfo {
                    state: clearing,
                    address: liqor_ctx.account_ctx.state.clearing,
                }
            }
        };

        if liqee_ctx.account_ctx.address == Default::default() {
            return Ok(());
        }
        let liqee_clearing = match self
            .clearings_map
            .get(&liqee_ctx.account_ctx.state.clearing)
        {
            Some(c) => ClearingInfo {
                state: c.clone(),
                address: liqee_ctx.account_ctx.state.clearing,
            },
            None => {
                let clearing = get_cypher_zero_copy_account::<Clearing>(
                    &self.rpc_client,
                    &liqee_ctx.account_ctx.state.clearing,
                )
                .await?;
                self.clearings_map
                    .insert(liqee_ctx.account_ctx.state.clearing, clearing.clone());
                ClearingInfo {
                    state: clearing,
                    address: liqee_ctx.account_ctx.state.clearing,
                }
            }
        };

        let liquidation_check = check_collateral(&cache_ctx, liqee_ctx, &liqee_clearing);

        info!("{:?}", liquidation_check);

        match liquidation_check.liquidation_type {
            LiquidationType::None => {
                info!(
                    "Liqee: {} - Not eligible for liquidation.",
                    liqee_ctx.account_ctx.state.authority,
                );
            }
            LiquidationType::Account { account } => {
                info!(
                    "Liqee: {} - Account: {} - Eligible for liquidation.",
                    liqee_ctx.account_ctx.state.authority, account
                );
                // check if the liquidator is actually able to liquidate
                if !check_can_liquidate(&cache_ctx, &liqor_ctx, &liqor_clearing) {
                    return Err(Error::LiquidatorCRatio);
                }

                match self
                    .process_account_liquidation(
                        &cache_ctx,
                        &liqor_ctx,
                        &liqor_clearing,
                        liqee_ctx,
                        &liqee_clearing,
                    )
                    .await
                {
                    Ok(sim_res) => {
                        info!(
                            "Chosen account liquidation simulation result: {:?}",
                            sim_res
                        );
                        let ixs =
                            get_liquidation_ix(&self.cypher_ctx, &liqor_ctx, liqee_ctx, &sim_res)
                                .await?;

                        let sig = match send_transactions(
                            &self.rpc_client,
                            ixs,
                            &self.user_info.signer,
                            false,
                            Some((1_400_000, 1)),
                            None,
                        )
                        .await
                        {
                            Ok(s) => s,
                            Err(e) => {
                                warn!("Failed to submit transaction.");
                                return Err(Error::Client(e));
                            }
                        };

                        info!(
                            "Successfully submitted liquidation. Transaction signtaure: {}",
                            sig.first().unwrap()
                        );
                    }
                    Err(e) => {
                        warn!("Something went wrong while performing account liquidation simulation. {:?}", e);
                    }
                }
            }
            LiquidationType::SubAccount { sub_account } => {
                info!(
                    "Liqee: {} - Account: {} - Sub Account: {} - Eligible for liquidation.",
                    liqee_ctx.account_ctx.state.authority,
                    liqee_ctx.account_ctx.address,
                    sub_account
                );
                // check if the liquidator is actually able to liquidate
                if !check_can_liquidate(&cache_ctx, &liqor_ctx, &liqor_clearing) {
                    return Err(Error::LiquidatorCRatio);
                }

                let (liqee_sub_account_idx, liqee_sub_account_ctx) = liqee_ctx
                    .sub_account_ctxs
                    .iter()
                    .enumerate()
                    .find(|(_, sa)| sa.address == sub_account)
                    .unwrap();

                match self
                    .process_sub_account_liquidation(
                        &cache_ctx,
                        &liqor_ctx,
                        &liqor_clearing,
                        liqee_ctx,
                        liqee_sub_account_ctx,
                        liqee_sub_account_idx,
                        &liqee_clearing,
                    )
                    .await
                {
                    Ok(results) => {
                        info!(
                            "Chosen sub account liquidation simulation result: {:?}",
                            results
                        );

                        let sim_res = get_highest_simulation_value(&cache_ctx, results);
                        info!("Chosen simulation result: {:?}", sim_res);
                        let ixs =
                            get_liquidation_ix(&self.cypher_ctx, &liqor_ctx, liqee_ctx, &sim_res)
                                .await?;

                        let sig = match send_transactions(
                            &self.rpc_client,
                            ixs,
                            &self.user_info.signer,
                            false,
                            Some((1_400_000, 1)),
                            None,
                        )
                        .await
                        {
                            Ok(s) => s,
                            Err(e) => {
                                warn!("Failed to submit transaction.");
                                return Err(Error::Client(e));
                            }
                        };

                        info!(
                            "Successfully submitted liquidation. Transaction signtaure: {}",
                            sig.first().unwrap()
                        );
                    }
                    Err(e) => {
                        warn!("Something went wrong while performing sub account liquidation simulation. {:?}", e);
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn process_account_liquidation(
        &self,
        cache_ctx: &CacheContext,
        liqor_ctx: &UserContext,
        liqor_clearing: &ClearingInfo,
        liqee_ctx: &UserContext,
        liqee_clearing: &ClearingInfo,
    ) -> Result<SimulationResult, Error> {
        // check which asset the liquidator is able to liquidate

        let mut possible_liqs = Vec::new();

        for (liqee_sub_account_idx, liqee_sub_account_ctx) in
            liqee_ctx.sub_account_ctxs.iter().enumerate()
        {
            match self
                .process_sub_account_liquidation(
                    cache_ctx,
                    liqor_ctx,
                    liqor_clearing,
                    liqee_ctx,
                    liqee_sub_account_ctx,
                    liqee_sub_account_idx,
                    liqee_clearing,
                )
                .await
            {
                Ok(results) => {
                    info!(
                        "Performed {} possible simulations for sub account: {}",
                        results.len(),
                        liqee_sub_account_ctx.address
                    );
                    if !results.is_empty() {
                        possible_liqs.extend(results);
                    }
                }
                Err(e) => {
                    warn!("Something went wrong while performing sub account liquidation simulation. {:?}", e);
                }
            };
        }

        Ok(get_highest_simulation_value(cache_ctx, possible_liqs))
    }

    pub async fn process_sub_account_liquidation(
        &self,
        cache_ctx: &CacheContext,
        liqor_ctx: &UserContext,
        liqor_clearing: &ClearingInfo,
        liqee_ctx: &UserContext,
        liqee_sub_account_ctx: &SubAccountContext,
        liqee_sub_account_idx: usize,
        liqee_clearing: &ClearingInfo,
    ) -> Result<Vec<SimulationResult>, Error> {
        // check which asset the liquidator is able to liquidate

        let mut simulation_results = Vec::new();

        let assets_value = liqee_sub_account_ctx
            .state
            .get_assets_value(
                cache_ctx.state.as_ref(),
                MarginCollateralRatioType::Maintenance,
            )
            .checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32)))
            .unwrap();
        let liabilities_value = liqee_sub_account_ctx
            .state
            .get_liabilities_value(
                cache_ctx.state.as_ref(),
                MarginCollateralRatioType::Maintenance,
            )
            .checked_mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32)))
            .unwrap();

        info!(
            "Liqee Sub Account: {} - Assets Value: {} - Liabilities Value: {}",
            liqee_sub_account_ctx.address, assets_value, liabilities_value
        );

        for position in liqee_sub_account_ctx.state.iter_position_slots() {
            // if it is a spot position and it is a borrow
            let (liqor_sub_account, liability_mint) = if position.spot.token_mint
                != Pubkey::default()
                && position.spot.position().is_negative()
            {
                // try to get the liqor sub account with position for this liability
                match liqor_ctx.get_sub_account_with_position(&position.spot.token_mint) {
                    Some(sa) => {
                        // let's get the position and see if it is big enough to cover the liability
                        match sa.get_spot_position(&position.spot.token_mint) {
                            Some(p) => {
                                info!("Liqor Liability Spot Position: {}", p.position());
                                if p.position() > position.spot.position().abs() {
                                    (Some(sa), position.spot.token_mint)
                                } else {
                                    (None, position.spot.token_mint)
                                }
                            }
                            None => (None, position.spot.token_mint),
                        }
                    }
                    None => (None, position.spot.token_mint),
                }
            } else {
                (None, Pubkey::default())
            };

            if liability_mint != Pubkey::default() {
                match liqor_sub_account {
                    Some(liqor_sub_account) => {
                        info!(
                            "Using liqor sub account for spot liq simulation: {}",
                            liqor_sub_account.address
                        );
                        // get the highest value position
                        let asset_spot_position =
                            get_highest_value_spot_position(cache_ctx, liqee_sub_account_ctx);

                        if asset_spot_position.token_mint != Pubkey::default() {
                            info!(
                                "Asset Spot Position: {} - Size: {}",
                                asset_spot_position.token_mint,
                                asset_spot_position.position(),
                            );
                            let liqor_sub_account_idx = liqor_ctx
                                .sub_account_ctxs
                                .iter()
                                .enumerate()
                                .find(|(_, sa)| sa.address == liqor_sub_account.address)
                                .map(|(idx, _)| idx)
                                .unwrap();

                            let simulation_ctx = SimulationContext {
                                cache: cache_ctx.clone(),
                                liqor: liqor_ctx.clone(),
                                liqor_clearing: liqor_clearing.clone(),
                                liqor_sub_account_idx,
                                liqee: liqee_ctx.clone(),
                                liqee_clearing: liqee_clearing.clone(),
                                liqee_sub_account_idx,
                            };

                            let asset_cache_idx = asset_spot_position.cache_index as usize;
                            let asset_cache = cache_ctx.state.get_price_cache(asset_cache_idx);

                            let liability_cache_idx = position.spot.cache_index as usize;
                            let liability_cache =
                                cache_ctx.state.get_price_cache(liability_cache_idx);

                            let simulation_args = SimulateLiquidationArgs {
                                assets_value,
                                liabilities_value,
                                asset: asset_spot_position.token_mint,
                                liability: liability_mint,
                                asset_cache_idx,
                                liability_cache_idx,
                                asset_decimals: asset_cache.decimals,
                                liability_decimals: liability_cache.decimals,
                                asset_price: asset_cache.oracle_price(),
                                liability_price: liability_cache.oracle_price(),
                                insurance_fund: I80F48::ZERO, // todo: change this, we need insurance fund for cases where liqee is bankrupt
                            };
                            match simulate_liquidation(SimulationType::Spot {
                                simulation_ctx,
                                simulation_args,
                            }) {
                                Ok(simulation_res) => {
                                    info!("Successfully performed simulation.");
                                    if simulation_res.repay_amount != I80F48::ZERO
                                        && simulation_res.liqee_asset_debit != 0
                                    {
                                        simulation_results.push(simulation_res);
                                    }
                                }
                                Err(e) => {
                                    warn!("There was an error performing simulation: {:?}", e);
                                }
                            };
                        }
                    }
                    None => {
                        warn!(
                            "Liquidator does not have position for token: {}",
                            liability_mint
                        );
                    }
                };
            }

            // if it is a perp position and it is a short
            let (liqor_sub_account, liability_market) = if position.derivative.market
                != Pubkey::default()
                && position.derivative.total_position().is_negative()
                && position.derivative.market_type == MarketType::PerpetualFuture
            {
                info!("Perpetual liability market: {}", position.derivative.market);
                // try to get the liqor sub account with position for this liability
                match liqor_ctx.get_sub_account_with_position(&position.derivative.market) {
                    Some(sa) => {
                        // let's get the position and see if it is big enough to cover the liability
                        match sa.get_derivative_position(&position.derivative.market) {
                            Some(p) => {
                                info!("Liqor Liability Perpetual Position: {}", p.total_position());
                                if p.total_position() > position.derivative.total_position().abs() {
                                    (Some(sa), position.derivative.market)
                                } else {
                                    (None, position.derivative.market)
                                }
                            }
                            None => (None, position.derivative.market),
                        }
                    }
                    None => (None, position.derivative.market),
                }
            } else {
                (None, Pubkey::default())
            };

            if liability_market != Pubkey::default() {
                match liqor_sub_account {
                    Some(liqor_sub_account) => {
                        // get the highest value position
                        let asset_perp_position =
                            get_highest_value_perp_position(cache_ctx, liqee_sub_account_ctx);

                        if asset_perp_position.market != Pubkey::default() {
                            let liqor_sub_account_idx = liqor_ctx
                                .sub_account_ctxs
                                .iter()
                                .enumerate()
                                .find(|(_, sa)| sa.address == liqor_sub_account.address)
                                .map(|(idx, _)| idx)
                                .unwrap();

                            let simulation_ctx = SimulationContext {
                                cache: cache_ctx.clone(),
                                liqor: liqor_ctx.clone(),
                                liqor_clearing: liqor_clearing.clone(),
                                liqor_sub_account_idx,
                                liqee: liqee_ctx.clone(),
                                liqee_clearing: liqee_clearing.clone(),
                                liqee_sub_account_idx,
                            };

                            let asset_cache_idx = asset_perp_position.cache_index as usize;
                            let asset_cache = cache_ctx.state.get_price_cache(asset_cache_idx);

                            let liability_cache_idx = position.spot.cache_index as usize;
                            let liability_cache =
                                cache_ctx.state.get_price_cache(liability_cache_idx);

                            let simulation_args = SimulateLiquidationArgs {
                                assets_value,
                                liabilities_value,
                                asset: asset_perp_position.market,
                                liability: liability_market,
                                asset_cache_idx,
                                liability_cache_idx,
                                asset_decimals: asset_cache.perp_decimals,
                                liability_decimals: liability_cache.perp_decimals,
                                asset_price: asset_cache.oracle_price(),
                                liability_price: liability_cache.oracle_price(),
                                insurance_fund: I80F48::ZERO, // todo: change this, we need insurance fund for cases where liqee is bankrupt
                            };
                            match simulate_liquidation(SimulationType::Perpetuals {
                                simulation_ctx,
                                simulation_args,
                            }) {
                                Ok(simulation_res) => {
                                    info!("Successfully performed simulation.");
                                    if simulation_res.repay_amount != I80F48::ZERO
                                        && simulation_res.liqee_asset_debit != 0
                                    {
                                        simulation_results.push(simulation_res);
                                    }
                                }
                                Err(e) => {
                                    warn!("There was an error performing simulation: {:?}", e);
                                }
                            };
                        }
                    }
                    None => {
                        warn!(
                            "Liquidator does not have position for market: {}",
                            liability_market
                        );
                    }
                };
            }

            // if it is a futures position and it is a short
            let (liqor_sub_account, liability_market) = if position.derivative.market
                != Pubkey::default()
                && position.derivative.total_position().is_negative()
                && position.derivative.market_type != MarketType::PerpetualFuture
            {
                info!("Future liability market: {}", position.derivative.market);
                // try to get the liqor sub account with position for this liability
                match liqor_ctx.get_sub_account_with_position(&position.derivative.market) {
                    Some(sa) => {
                        // let's get the position and see if it is big enough to cover the liability
                        match sa.get_derivative_position(&position.derivative.market) {
                            Some(p) => {
                                info!("Liqor Liability Future Position: {}", p.total_position());
                                if p.total_position() > position.derivative.total_position().abs() {
                                    (Some(sa), position.derivative.market)
                                } else {
                                    (None, position.derivative.market)
                                }
                            }
                            None => (None, position.derivative.market),
                        }
                    }
                    None => (None, position.derivative.market),
                }
            } else {
                (None, Pubkey::default())
            };

            if liability_market != Pubkey::default() {
                match liqor_sub_account {
                    Some(liqor_sub_account) => {
                        // get the highest value position
                        let asset_futures_position =
                            get_highest_value_futures_position(cache_ctx, liqee_sub_account_ctx);

                        if asset_futures_position.market != Pubkey::default() {
                            let liqor_sub_account_idx = liqor_ctx
                                .sub_account_ctxs
                                .iter()
                                .enumerate()
                                .find(|(_, sa)| sa.address == liqor_sub_account.address)
                                .map(|(idx, _)| idx)
                                .unwrap();

                            let simulation_ctx = SimulationContext {
                                cache: cache_ctx.clone(),
                                liqor: liqor_ctx.clone(),
                                liqor_clearing: liqor_clearing.clone(),
                                liqor_sub_account_idx,
                                liqee: liqee_ctx.clone(),
                                liqee_clearing: liqee_clearing.clone(),
                                liqee_sub_account_idx,
                            };

                            let asset_cache_idx = asset_futures_position.cache_index as usize;
                            let asset_cache = cache_ctx.state.get_price_cache(asset_cache_idx);

                            let liability_cache_idx = position.spot.cache_index as usize;
                            let liability_cache =
                                cache_ctx.state.get_price_cache(liability_cache_idx);

                            let simulation_args = SimulateLiquidationArgs {
                                assets_value,
                                liabilities_value,
                                asset: asset_futures_position.market,
                                liability: liability_market,
                                asset_cache_idx,
                                liability_cache_idx,
                                asset_decimals: asset_cache.perp_decimals,
                                liability_decimals: liability_cache.perp_decimals,
                                asset_price: asset_cache.oracle_price(),
                                liability_price: liability_cache.oracle_price(),
                                insurance_fund: I80F48::ZERO, // todo: change this, we need insurance fund for cases where liqee is bankrupt
                            };
                            match simulate_liquidation(SimulationType::Futures {
                                simulation_ctx,
                                simulation_args,
                            }) {
                                Ok(simulation_res) => {
                                    info!("Successfully performed simulation.");
                                    if simulation_res.repay_amount != I80F48::ZERO
                                        && simulation_res.liqee_asset_debit != 0
                                    {
                                        simulation_results.push(simulation_res);
                                    }
                                }
                                Err(e) => {
                                    warn!("There was an error performing simulation: {:?}", e);
                                }
                            };
                        }
                    }
                    None => {
                        warn!(
                            "Liquidator does not have position for market: {}",
                            liability_market
                        );
                    }
                };
            }
        }

        Ok(simulation_results)
    }
}

fn get_highest_value_spot_position(
    cache_ctx: &CacheContext,
    sub_account_ctx: &SubAccountContext,
) -> SpotPosition {
    let mut spot_position = SpotPosition::default();
    let mut highest_value = I80F48::ZERO;

    for position in sub_account_ctx.state.iter_position_slots() {
        if position.spot.token_mint != Pubkey::default() && position.spot.position().is_positive() {
            let cache = cache_ctx
                .state
                .get_price_cache(position.spot.cache_index as usize);
            let position_value =
                adjust_decimals(position.spot.total_position(cache), cache.decimals)
                    .checked_mul(cache.oracle_price())
                    .unwrap();
            info!(
                "Spot Position: {} - Value: {}",
                position.spot.token_mint, position_value
            );
            if position_value > highest_value {
                highest_value = position_value;
                spot_position = position.spot;
            }
        }
    }

    spot_position
}

fn get_highest_value_perp_position(
    cache_ctx: &CacheContext,
    sub_account_ctx: &SubAccountContext,
) -> DerivativePosition {
    let mut deriv_position = DerivativePosition::default();
    let mut highest_value = I80F48::ZERO;

    for position in sub_account_ctx.state.iter_position_slots() {
        if position.derivative.market != Pubkey::default()
            && position.derivative.total_position().is_positive()
            && position.derivative.market_type == MarketType::PerpetualFuture
        {
            let cache = cache_ctx
                .state
                .get_price_cache(position.derivative.cache_index as usize);
            let position_value =
                adjust_decimals(position.derivative.total_position(), cache.decimals)
                    .checked_mul(cache.oracle_price())
                    .unwrap();
            info!(
                "Perpetual Position: {} - Value: {}",
                position.spot.token_mint, position_value
            );
            if position_value > highest_value {
                highest_value = position_value;
                deriv_position = position.derivative;
            }
        }
    }

    deriv_position
}

fn get_highest_value_futures_position(
    cache_ctx: &CacheContext,
    sub_account_ctx: &SubAccountContext,
) -> DerivativePosition {
    let mut deriv_position = DerivativePosition::default();
    let mut highest_value = I80F48::ZERO;

    for position in sub_account_ctx.state.iter_position_slots() {
        if position.derivative.market != Pubkey::default()
            && position.derivative.total_position().is_positive()
            && position.derivative.market_type != MarketType::PerpetualFuture
        {
            let cache = cache_ctx
                .state
                .get_price_cache(position.derivative.cache_index as usize);
            let position_value =
                adjust_decimals(position.derivative.total_position(), cache.decimals)
                    .checked_mul(cache.oracle_price())
                    .unwrap();
            info!(
                "Future Position: {} - Value: {}",
                position.spot.token_mint, position_value
            );
            if position_value > highest_value {
                highest_value = position_value;
                deriv_position = position.derivative;
            }
        }
    }

    deriv_position
}

async fn get_liquidation_ix(
    cypher_context: &CypherContext,
    liqor_ctx: &UserContext,
    liqee_ctx: &UserContext,
    simulation_result: &SimulationResult,
) -> Result<Vec<Instruction>, Error> {
    match &simulation_result.simulation_type {
        SimulationType::Futures {
            simulation_ctx,
            simulation_args,
        } => {
            let liqor_sub_account_ctx =
                &liqor_ctx.sub_account_ctxs[simulation_ctx.liqor_sub_account_idx];
            let liqee_sub_account_ctx =
                &liqee_ctx.sub_account_ctxs[simulation_ctx.liqee_sub_account_idx];
            get_liquidate_futures_position_ix(
                liqor_ctx,
                liqor_sub_account_ctx,
                liqee_ctx,
                liqee_sub_account_ctx,
                simulation_args,
            )
        }
        SimulationType::Perpetuals {
            simulation_ctx,
            simulation_args,
        } => {
            let liqor_sub_account_ctx =
                &liqor_ctx.sub_account_ctxs[simulation_ctx.liqor_sub_account_idx];
            let liqee_sub_account_ctx =
                &liqee_ctx.sub_account_ctxs[simulation_ctx.liqee_sub_account_idx];
            get_liquidate_perp_position_ix(
                liqor_ctx,
                liqor_sub_account_ctx,
                liqee_ctx,
                liqee_sub_account_ctx,
                simulation_args,
            )
        }
        SimulationType::Spot {
            simulation_ctx,
            simulation_args,
        } => {
            let liqor_sub_account_ctx =
                &liqor_ctx.sub_account_ctxs[simulation_ctx.liqor_sub_account_idx];
            let liqee_sub_account_ctx =
                &liqee_ctx.sub_account_ctxs[simulation_ctx.liqee_sub_account_idx];
            Ok(get_liquidate_spot_position_ix(
                cypher_context,
                liqor_ctx,
                liqor_sub_account_ctx,
                liqee_ctx,
                liqee_sub_account_ctx,
                simulation_args,
            )
            .await)
        }
    }
}

async fn get_liquidate_spot_position_ix(
    cypher_context: &CypherContext,
    liqor_ctx: &UserContext,
    liqor_sub_account_ctx: &SubAccountContext,
    liqee_ctx: &UserContext,
    liqee_sub_account_ctx: &SubAccountContext,
    simulation_args: &SimulateLiquidationArgs,
) -> Vec<Instruction> {
    let pools = cypher_context.pools.read().await;

    let asset_pool = pools
        .iter()
        .find(|p| p.state.token_mint == simulation_args.asset)
        .unwrap();
    let asset_pool_node = asset_pool.pool_nodes.first().unwrap();
    let liability_pool = pools
        .iter()
        .find(|p| p.state.token_mint == simulation_args.liability)
        .unwrap();
    let liability_pool_node = liability_pool.pool_nodes.first().unwrap();

    vec![
        update_account_margin_ix(
            &cache_account::id(),
            &liqee_ctx.account_ctx.address,
            &liqor_ctx.account_ctx.state.authority,
            &liqee_ctx
                .account_ctx
                .state
                .sub_account_caches
                .iter()
                .filter(|c| c.sub_account != Pubkey::default())
                .map(|sac| sac.sub_account)
                .collect::<Vec<_>>(),
        ),
        liquidate_spot_position_ix(
            &cache_account::id(),
            &liqor_ctx.account_ctx.state.clearing,
            &liqor_ctx.account_ctx.address,
            &liqor_sub_account_ctx.address,
            &liqee_ctx.account_ctx.state.clearing,
            &liqee_ctx.account_ctx.address,
            &liqee_sub_account_ctx.address,
            &simulation_args.asset,
            &asset_pool_node.address,
            &simulation_args.liability,
            &liability_pool.address,
            &liability_pool_node.address,
            &liqor_ctx.account_ctx.state.authority,
        ),
    ]
}

fn get_liquidate_perp_position_ix(
    liqor_ctx: &UserContext,
    liqor_sub_account_ctx: &SubAccountContext,
    liqee_ctx: &UserContext,
    liqee_sub_account_ctx: &SubAccountContext,
    simulation_args: &SimulateLiquidationArgs,
) -> Result<Vec<Instruction>, Error> {
    let encoded_quote_pool_name = encode_string("USDC");
    let (quote_pool, _) = derive_pool_address(&encoded_quote_pool_name);
    let (quote_pool_node, _) = derive_pool_node_address(&quote_pool, 0); // default to first node for now

    let mut ixs = vec![update_account_margin_ix(
        &cache_account::id(),
        &liqee_ctx.account_ctx.address,
        &liqor_ctx.account_ctx.state.authority,
        &liqee_ctx
            .account_ctx
            .state
            .sub_account_caches
            .iter()
            .filter(|c| c.sub_account != Pubkey::default())
            .map(|sac| sac.sub_account)
            .collect::<Vec<_>>(),
    )];

    if simulation_args.asset == quote_mint::id() {
        ixs.push(liquidate_perp_position_ix(
            &cache_account::id(),
            &liqor_ctx.account_ctx.state.clearing,
            &liqor_ctx.account_ctx.address,
            &liqor_sub_account_ctx.address,
            &liqee_ctx.account_ctx.state.clearing,
            &liqee_ctx.account_ctx.address,
            &liqee_sub_account_ctx.address,
            &simulation_args.asset,
            &Pubkey::default(),
            &simulation_args.liability,
            &simulation_args.liability,
            &quote_pool,
            &quote_pool_node,
            &liqor_ctx.account_ctx.state.authority,
        )?);
        return Ok(ixs);
    }

    if simulation_args.liability == quote_mint::id() {
        ixs.push(liquidate_perp_position_ix(
            &cache_account::id(),
            &liqor_ctx.account_ctx.state.clearing,
            &liqor_ctx.account_ctx.address,
            &liqor_sub_account_ctx.address,
            &liqee_ctx.account_ctx.state.clearing,
            &liqee_ctx.account_ctx.address,
            &liqee_sub_account_ctx.address,
            &simulation_args.asset,
            &simulation_args.asset,
            &simulation_args.liability,
            &Pubkey::default(),
            &quote_pool,
            &quote_pool_node,
            &liqor_ctx.account_ctx.state.authority,
        )?);
        return Ok(ixs);
    }
    ixs.push(liquidate_perp_position_ix(
        &cache_account::id(),
        &liqor_ctx.account_ctx.state.clearing,
        &liqor_ctx.account_ctx.address,
        &liqor_sub_account_ctx.address,
        &liqee_ctx.account_ctx.state.clearing,
        &liqee_ctx.account_ctx.address,
        &liqee_sub_account_ctx.address,
        &simulation_args.asset,
        &simulation_args.asset,
        &simulation_args.liability,
        &simulation_args.liability,
        &quote_pool,
        &quote_pool_node,
        &liqor_ctx.account_ctx.state.authority,
    )?);
    Ok(ixs)
}

fn get_liquidate_futures_position_ix(
    liqor_ctx: &UserContext,
    liqor_sub_account_ctx: &SubAccountContext,
    liqee_ctx: &UserContext,
    liqee_sub_account_ctx: &SubAccountContext,
    simulation_args: &SimulateLiquidationArgs,
) -> Result<Vec<Instruction>, Error> {
    let encoded_quote_pool_name = encode_string("USDC");
    let (quote_pool, _) = derive_pool_address(&encoded_quote_pool_name);
    let (quote_pool_node, _) = derive_pool_node_address(&quote_pool, 0); // default to first node for now

    let mut ixs = vec![update_account_margin_ix(
        &cache_account::id(),
        &liqee_ctx.account_ctx.address,
        &liqor_ctx.account_ctx.state.authority,
        &liqee_ctx
            .account_ctx
            .state
            .sub_account_caches
            .iter()
            .filter(|c| c.sub_account != Pubkey::default())
            .map(|sac| sac.sub_account)
            .collect::<Vec<_>>(),
    )];

    if simulation_args.asset == quote_mint::id() {
        ixs.push(liquidate_futures_position_ix(
            &cache_account::id(),
            &liqor_ctx.account_ctx.state.clearing,
            &liqor_ctx.account_ctx.address,
            &liqor_sub_account_ctx.address,
            &liqee_ctx.account_ctx.state.clearing,
            &liqee_ctx.account_ctx.address,
            &liqee_sub_account_ctx.address,
            &simulation_args.asset,
            &Pubkey::default(),
            &simulation_args.liability,
            &simulation_args.liability,
            &quote_pool,
            &quote_pool_node,
            &liqor_ctx.account_ctx.state.authority,
        )?);
        return Ok(ixs);
    }

    if simulation_args.liability == quote_mint::id() {
        ixs.push(liquidate_futures_position_ix(
            &cache_account::id(),
            &liqor_ctx.account_ctx.state.clearing,
            &liqor_ctx.account_ctx.address,
            &liqor_sub_account_ctx.address,
            &liqee_ctx.account_ctx.state.clearing,
            &liqee_ctx.account_ctx.address,
            &liqee_sub_account_ctx.address,
            &simulation_args.asset,
            &simulation_args.asset,
            &simulation_args.liability,
            &Pubkey::default(),
            &quote_pool,
            &quote_pool_node,
            &liqor_ctx.account_ctx.state.authority,
        )?);
        return Ok(ixs);
    }
    ixs.push(liquidate_futures_position_ix(
        &cache_account::id(),
        &liqor_ctx.account_ctx.state.clearing,
        &liqor_ctx.account_ctx.address,
        &liqor_sub_account_ctx.address,
        &liqee_ctx.account_ctx.state.clearing,
        &liqee_ctx.account_ctx.address,
        &liqee_sub_account_ctx.address,
        &simulation_args.asset,
        &simulation_args.asset,
        &simulation_args.liability,
        &simulation_args.liability,
        &quote_pool,
        &quote_pool_node,
        &liqor_ctx.account_ctx.state.authority,
    )?);
    Ok(ixs)
}

fn get_highest_simulation_value(
    cache_context: &CacheContext,
    simulations: Vec<SimulationResult>,
) -> SimulationResult {
    let mut best_simulation = SimulationResult::default();
    let mut highest_value = I80F48::ZERO;

    for simulation in simulations {
        match simulation.simulation_type {
            SimulationType::Futures {
                ref simulation_ctx,
                ref simulation_args,
            } => {
                let asset_cache = cache_context
                    .state
                    .get_price_cache(simulation_args.asset_cache_idx);
                let liqee_sub_account =
                    &simulation_ctx.liqee.sub_account_ctxs[simulation_ctx.liqee_sub_account_idx];
                let position_idx = liqee_sub_account
                    .state
                    .get_position_idx(&simulation_args.asset, false)
                    .unwrap();
                let position = liqee_sub_account
                    .state
                    .get_derivative_position(position_idx);
                let asset_value =
                    adjust_decimals(position.total_position(), simulation_args.asset_decimals)
                        .checked_mul(asset_cache.oracle_price())
                        .unwrap();

                if asset_value > highest_value {
                    highest_value = asset_value;
                    best_simulation = simulation.clone();
                }
            }
            SimulationType::Perpetuals {
                ref simulation_ctx,
                ref simulation_args,
            } => {
                let asset_cache = cache_context
                    .state
                    .get_price_cache(simulation_args.asset_cache_idx);
                let liqee_sub_account =
                    &simulation_ctx.liqee.sub_account_ctxs[simulation_ctx.liqee_sub_account_idx];
                let position_idx = liqee_sub_account
                    .state
                    .get_position_idx(&simulation_args.asset, false)
                    .unwrap();
                let position = liqee_sub_account
                    .state
                    .get_derivative_position(position_idx);
                let asset_value =
                    adjust_decimals(position.total_position(), simulation_args.asset_decimals)
                        .checked_mul(asset_cache.oracle_price())
                        .unwrap();

                if asset_value > highest_value {
                    highest_value = asset_value;
                    best_simulation = simulation.clone();
                }
            }
            SimulationType::Spot {
                ref simulation_ctx,
                ref simulation_args,
            } => {
                let asset_cache = cache_context
                    .state
                    .get_price_cache(simulation_args.asset_cache_idx);
                let liqee_sub_account =
                    &simulation_ctx.liqee.sub_account_ctxs[simulation_ctx.liqee_sub_account_idx];
                let position_idx = liqee_sub_account
                    .state
                    .get_position_idx(&simulation_args.asset, true)
                    .unwrap();
                let position = liqee_sub_account.state.get_spot_position(position_idx);
                let asset_value = adjust_decimals(
                    position.total_position(asset_cache),
                    simulation_args.asset_decimals,
                )
                .checked_mul(asset_cache.oracle_price())
                .unwrap();

                if asset_value > highest_value {
                    highest_value = asset_value;
                    best_simulation = simulation.clone();
                }
            }
        }
    }

    best_simulation
}
