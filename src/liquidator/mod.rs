pub mod accounts;
pub mod check;
pub mod config;
pub mod error;
pub mod info;
pub mod simulation;

use anchor_lang::prelude::Pubkey;
use cypher_client::Clearing;
use cypher_utils::{
    contexts::{CacheContext, SubAccountContext, UserContext},
    utils::get_cypher_zero_copy_account,
};
use dashmap::DashMap;
use log::{info, warn};
use solana_client::nonblocking::rpc_client::RpcClient;

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
    },
};

pub struct Liquidator {
    rpc_client: Arc<RpcClient>,
    shutdown_sender: Arc<Sender<bool>>,
    global_context_builder: Arc<dyn ContextBuilder<Output = GlobalContext>>,
    update_sender: Arc<Sender<UserContext>>,
    user_info: UserInfo,
    clearings_map: DashMap<Pubkey, Box<Clearing>>,
    cache_ctx: RwLock<CacheContext>,
    liqor_ctx: RwLock<UserContext>,
}

impl Liquidator {
    pub fn new(
        rpc_client: Arc<RpcClient>,
        global_context_builder: Arc<dyn ContextBuilder<Output = GlobalContext>>,
        update_sender: Arc<Sender<UserContext>>,
        shutdown_sender: Arc<Sender<bool>>,
        user_info: UserInfo,
    ) -> Self {
        Self {
            rpc_client,
            global_context_builder,
            update_sender,
            shutdown_sender,
            user_info,
            clearings_map: DashMap::new(),
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
                            let start = SystemTime::now();
                            match self.process(&user_ctx).await {
                                Ok(()) => info!("Successfully processed user context update."),
                                Err(e) => {
                                    warn!("There was an error processing account update. Error: {:?}", e);
                                }
                            };
                            let delta = start.elapsed().unwrap();
                            info!("Time elapsed: {:?}", delta);
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
                    state: clearing.clone(),
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
                    state: clearing.clone(),
                    address: liqee_ctx.account_ctx.state.clearing,
                }
            }
        };

        let liquidation_check = check_collateral(&cache_ctx, &liqee_ctx, &liqee_clearing);

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

                let (_liqor_can_liq, _asset_info, _liability_info) = self
                    .process_account_liquidation(
                        &cache_ctx,
                        &liqor_ctx,
                        &liqor_clearing,
                        &liqee_ctx,
                        &liqee_clearing,
                    )
                    .await?;
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

                let liqee_sub_account_ctx = liqee_ctx
                    .sub_account_ctxs
                    .iter()
                    .find(|sa| sa.address == sub_account)
                    .unwrap();

                let (_liqor_can_liq, _asset_info, _liability_info) = self
                    .process_sub_account_liquidation(
                        &cache_ctx,
                        &liqor_ctx,
                        &liqor_clearing,
                        liqee_sub_account_ctx,
                        &liqee_clearing,
                    )
                    .await?;
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
    ) -> Result<(bool, usize, usize), Error> {
        // check which asset the liquidator is able to liquidate

        let mut possible_liqs = Vec::new();

        for liqee_sub_account_ctx in liqee_ctx.sub_account_ctxs.iter() {
            let res = self
                .process_sub_account_liquidation(
                    cache_ctx,
                    liqor_ctx,
                    liqor_clearing,
                    liqee_sub_account_ctx,
                    liqee_clearing,
                )
                .await?;
            possible_liqs.push(res);
        }

        // out of all possible liquidations, attempt the one with the least impact

        Ok((false, usize::default(), usize::default()))
    }

    pub async fn process_sub_account_liquidation(
        &self,
        _cache_ctx: &CacheContext,
        _liqor_ctx: &UserContext,
        _liqor_clearing: &ClearingInfo,
        _liqee_sub_account_ctx: &SubAccountContext,
        _liqee_clearing: &ClearingInfo,
    ) -> Result<(bool, usize, usize), Error> {
        // check which asset the liquidator is able to liquidate

        Ok((false, usize::default(), usize::default()))
    }
}
