#![allow(dead_code)]
use cypher_client::{
    utils::{
        derive_orders_account_address, derive_spot_open_orders_address, gen_dex_vault_signer_key,
    },
    FuturesMarket, PerpetualMarket,
};
use cypher_utils::{
    accounts_cache::AccountsCache,
    contexts::{CypherContext, MarketContext, PoolContext, SpotMarketContext},
};
use fixed::types::I80F48;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use solana_client::nonblocking::rpc_client::RpcClient;
use std::{str::from_utf8, sync::Arc};
use tokio::sync::broadcast::Sender;

use crate::{
    common::{
        context::{
            builder::ContextBuilder, manager::ContextManager, ContextInfo, ExecutionContext,
            GlobalContext, OperationContext,
        },
        hedger::HedgerPulseResult,
        info::{
            Accounts, FuturesMarketInfo, MarketMetadata, PerpMarketInfo, SpotMarketInfo, UserInfo,
        },
        inventory::InventoryManager,
        maker::Maker,
        oracle::{OracleInfo, OracleProvider},
        strategy::Strategy,
    },
    config::{Config, ConfigError},
    context::{
        builders::{derivatives::DerivativeContextBuilder, spot::SpotContextBuilder},
        cypher_manager::CypherExecutionContextManager,
    },
    market_maker::{
        constants::BPS_UNIT,
        hedging::{futures::FuturesHedger, perps::PerpsHedger, spot::SpotHedger},
        inventory::ShapeFunctionInventoryManager,
        making::{futures::FuturesMaker, perps::PerpsMaker, spot::SpotMaker},
    },
    oracle::cypher::CypherOracleProvider,
    utils::accounts::{get_or_create_orders_account, get_or_create_spot_orders_account},
};

use super::error::Error;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketMakerConfig {
    pub maker_config: MakerConfig,
    pub inventory_config: InventoryConfig,
    pub hedger_config: HedgerConfig,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MakerConfig {
    /// the maximum units to quote
    pub max_quote: f64,
    /// the target bid/ask spread in bps
    pub spread: u16,
    /// the number of order layers per side
    pub layers: u8,
    /// the spacing between order layers in bps
    pub spacing_bps: u16,
    /// the time in force value for the orders
    pub time_in_force: u64,
    /// the symbol of the market to make
    pub symbol: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryConfig {
    /// the inventory function shape numerator
    pub shape_numerator: u64,
    /// the inventory function shape denominator
    pub shape_denominator: u64,
    /// the base exponent for the inventory function
    pub exp_base: u32,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct HedgerConfig {
    /// the symbol of the market to hedge
    pub symbol: String,
}

/// Gets the [`ContextInfo`] for the given symbol, which allows much easier construction of necessary components.
pub async fn get_context_info(
    rpc_client: Arc<RpcClient>,
    ctx: &CypherContext,
    user_info: &UserInfo,
    symbol: &str,
) -> Result<ContextInfo, Error> {
    info!("Preparing context info for {}..", symbol);

    let quote_pool = get_pool_from_symbol(ctx, "USDC").await.unwrap();

    let ctx_info = if symbol.contains("-PERP") {
        // get perp market from symbol
        let market_ctx = match get_perp_market_from_symbol(ctx, symbol).await {
            Ok(ctx) => ctx,
            Err(e) => {
                return Err(e);
            }
        };
        // get or create deriv orders account
        let orders_account =
            derive_orders_account_address(&market_ctx.address, &user_info.master_account).0;
        match get_or_create_orders_account(
            &rpc_client,
            &user_info.signer,
            &user_info.master_account,
            &market_ctx.address,
            &orders_account,
        )
        .await
        {
            Ok(_) => (),
            Err(e) => {
                warn!(
                    "There was an error getting or creating derivatives open orders account: {}",
                    e.to_string()
                );
                return Err(Error::ClientError(e));
            }
        };
        ContextInfo {
            symbol: symbol.to_string(),
            user_accounts: user_info.clone(),
            market_metadata: MarketMetadata {
                decimals: market_ctx.state.inner.config.decimals,
                cache_index: market_ctx.state.inner.config.cache_index,
                base_multiplier: market_ctx.state.inner.base_multiplier,
                quote_multiplier: market_ctx.state.inner.quote_multiplier,
            },
            context_accounts: Accounts::Perpetuals(PerpMarketInfo {
                state: *market_ctx.state.as_ref(),
                market: market_ctx.address,
                orderbook: market_ctx.state.inner.orderbook,
                bids: market_ctx.state.inner.bids,
                asks: market_ctx.state.inner.asks,
                event_queue: market_ctx.state.inner.event_queue,
                orders: orders_account,
                quote_pool: quote_pool.address,
                quote_pool_nodes: quote_pool
                    .pool_nodes
                    .iter()
                    .map(|pn| pn.address)
                    .collect::<Vec<_>>(),
            }),
        }
    } else if symbol.contains("1!") {
        // get futures market from symbol
        let market_ctx = match get_futures_market_from_symbol(ctx, symbol).await {
            Ok(ctx) => ctx,
            Err(e) => {
                return Err(e);
            }
        };
        // get or create deriv orders account
        let orders_account =
            derive_orders_account_address(&market_ctx.address, &user_info.master_account).0;
        match get_or_create_orders_account(
            &rpc_client,
            &user_info.signer,
            &user_info.master_account,
            &market_ctx.address,
            &orders_account,
        )
        .await
        {
            Ok(_) => (),
            Err(e) => {
                warn!(
                    "There was an error getting or creating derivatives open orders account: {}",
                    e.to_string()
                );
                return Err(Error::ClientError(e));
            }
        };
        ContextInfo {
            symbol: symbol.to_string(),
            user_accounts: user_info.clone(),
            market_metadata: MarketMetadata {
                decimals: market_ctx.state.inner.config.decimals,
                cache_index: market_ctx.state.inner.config.cache_index,
                base_multiplier: market_ctx.state.inner.base_multiplier,
                quote_multiplier: market_ctx.state.inner.quote_multiplier,
            },
            context_accounts: Accounts::Futures(FuturesMarketInfo {
                state: *market_ctx.state.as_ref(),
                market: market_ctx.address,
                orderbook: market_ctx.state.inner.orderbook,
                bids: market_ctx.state.inner.bids,
                asks: market_ctx.state.inner.asks,
                event_queue: market_ctx.state.inner.event_queue,
                price_history: market_ctx.state.inner.price_history,
                orders: orders_account,
                quote_pool: quote_pool.address,
                quote_pool_nodes: quote_pool
                    .pool_nodes
                    .iter()
                    .map(|pn| pn.address)
                    .collect::<Vec<_>>(),
            }),
        }
    } else {
        // get spot market from symbol
        let (spot_market, pool) = match get_spot_market_and_pool_from_symbol(ctx, symbol).await {
            Ok(ctx) => ctx,
            Err(e) => {
                return Err(e);
            }
        };
        // get or create spot open orders account
        let orders_account = derive_spot_open_orders_address(
            &spot_market.address,
            &user_info.master_account,
            &user_info.sub_account,
        )
        .0;
        match get_or_create_spot_orders_account(
            &rpc_client,
            &user_info.signer,
            &user_info.master_account,
            &user_info.sub_account,
            &spot_market.address,
            &pool.address,
            &pool.state.token_mint,
            &orders_account,
        )
        .await
        {
            Ok(_) => (),
            Err(e) => {
                warn!(
                    "There was an error getting or creating spot open orders account: {}",
                    e.to_string()
                );
                return Err(Error::ClientError(e));
            }
        };
        let dex_vault_signer =
            gen_dex_vault_signer_key(spot_market.state.vault_signer_nonce, &spot_market.address)
                .unwrap();
        ContextInfo {
            symbol: symbol.to_string(),
            user_accounts: user_info.clone(),
            market_metadata: MarketMetadata {
                decimals: pool.state.config.decimals,
                cache_index: pool.state.config.cache_index,
                base_multiplier: spot_market.state.coin_lot_size,
                quote_multiplier: spot_market.state.pc_lot_size,
            },
            context_accounts: Accounts::Spot(SpotMarketInfo {
                state: spot_market.state,
                market: spot_market.address,
                bids: spot_market.bids,
                asks: spot_market.asks,
                event_queue: spot_market.event_queue,
                request_queue: spot_market.request_queue,
                asset_mint: spot_market.base_mint,
                asset_vault: spot_market.base_vault,
                asset_vault_signer: pool.pool_nodes.first().unwrap().state.vault_signer, // todo: this should be done differently
                quote_vault: spot_market.quote_vault,
                quote_vault_signer: quote_pool.pool_nodes.first().unwrap().state.vault_signer, // todo: this should be done differently
                dex_coin_vault: spot_market.base_vault,
                dex_pc_vault: spot_market.quote_vault,
                dex_vault_signer,
                open_orders: orders_account,
                quote_pool: quote_pool.address,
                quote_pool_nodes: quote_pool
                    .pool_nodes
                    .iter()
                    .map(|pn| pn.address)
                    .collect::<Vec<_>>(),
                asset_pool: pool.address,
                asset_pool_nodes: pool
                    .pool_nodes
                    .iter()
                    .map(|pn| pn.address)
                    .collect::<Vec<_>>(),
            }),
        }
    };

    Ok(ctx_info)
}

/// Gets the appropriate [`Maker`] for the given config.
pub async fn get_maker_from_config(
    rpc_client: &Arc<RpcClient>,
    shutdown_sender: Arc<Sender<bool>>,
    context_sender: Arc<Sender<ExecutionContext>>,
    cypher_ctx: &CypherContext,
    context_info: &ContextInfo,
    config: &Config<MarketMakerConfig>,
) -> Result<
    Arc<dyn Maker<Input = ExecutionContext, InventoryManagerInput = GlobalContext> + Send>,
    Error,
> {
    info!("Preparing Maker for {}.", context_info.symbol);

    info!(
        "Inventory management - Max quote: {} - Target Spread: {} bps - Layers: {} - Spacing: {} bps",
        config.inner.maker_config.max_quote,
        config.inner.maker_config.spread,
        config.inner.maker_config.layers,
        config.inner.maker_config.spacing_bps
    );
    let decimals = get_decimals_for_symbol(cypher_ctx, context_info.symbol.as_str()).await?;

    let market_identifier = match &context_info.context_accounts {
        Accounts::Futures(f) => f.market,
        Accounts::Perpetuals(p) => p.market,
        Accounts::Spot(s) => s.asset_mint,
    };

    let is_derivative = match &context_info.context_accounts {
        Accounts::Futures(_) => true,
        Accounts::Perpetuals(_) => true,
        Accounts::Spot(_) => false,
    };

    let inventory_mngr: Arc<dyn InventoryManager<Input = GlobalContext>> =
        Arc::new(ShapeFunctionInventoryManager::new(
            market_identifier,
            is_derivative,
            decimals,
            config.inner.inventory_config.exp_base,
            I80F48::from_num::<f64>(config.inner.maker_config.max_quote),
            I80F48::from(config.inner.inventory_config.shape_numerator),
            I80F48::from(config.inner.inventory_config.shape_denominator),
            I80F48::from(BPS_UNIT)
                .checked_add(I80F48::from(config.inner.maker_config.spread))
                .and_then(|n| n.checked_div(I80F48::from(BPS_UNIT)))
                .unwrap(),
        ));

    let maker: Arc<
        dyn Maker<Input = ExecutionContext, InventoryManagerInput = GlobalContext> + Send,
    > = match &context_info.context_accounts {
        Accounts::Futures(f) => Arc::new(FuturesMaker::new(
            rpc_client.clone(),
            context_info.user_accounts.signer.clone(),
            inventory_mngr,
            shutdown_sender.clone(),
            context_sender.clone(),
            context_info.user_accounts.clone(),
            f.clone(),
            context_info.market_metadata.clone(),
            config.inner.maker_config.layers as usize,
            config.inner.maker_config.spacing_bps,
            config.inner.maker_config.time_in_force,
            context_info.symbol.to_string(),
        )),
        Accounts::Perpetuals(p) => Arc::new(PerpsMaker::new(
            rpc_client.clone(),
            context_info.user_accounts.signer.clone(),
            inventory_mngr,
            shutdown_sender.clone(),
            context_sender.clone(),
            context_info.user_accounts.clone(),
            p.clone(),
            context_info.market_metadata.clone(),
            config.inner.maker_config.layers as usize,
            config.inner.maker_config.spacing_bps,
            config.inner.maker_config.time_in_force,
            context_info.symbol.to_string(),
        )),
        Accounts::Spot(s) => Arc::new(SpotMaker::new(
            rpc_client.clone(),
            context_info.user_accounts.signer.clone(),
            inventory_mngr,
            shutdown_sender.clone(),
            context_sender.clone(),
            context_info.user_accounts.clone(),
            s.clone(),
            context_info.market_metadata.clone(),
            config.inner.maker_config.layers as usize,
            config.inner.maker_config.spacing_bps,
            config.inner.maker_config.time_in_force,
            context_info.symbol.to_string(),
        )),
    };

    Ok(maker)
}

/// Gets the appropriate [`Hedger`] for the given config.
pub fn get_hedger_from_config(
    context_info: &ContextInfo,
) -> Result<Arc<dyn Strategy<Input = ExecutionContext, Output = HedgerPulseResult>>, Error> {
    info!("Preparing Hedger for {}", context_info.symbol);

    let hedger: Arc<dyn Strategy<Input = ExecutionContext, Output = HedgerPulseResult>> =
        match &context_info.context_accounts {
            Accounts::Futures(_f) => Arc::new(FuturesHedger::new(context_info.symbol.to_string())),
            Accounts::Perpetuals(_p) => Arc::new(PerpsHedger::new(context_info.symbol.to_string())),
            Accounts::Spot(_s) => Arc::new(SpotHedger::new(context_info.symbol.to_string())),
        };

    Ok(hedger)
}

/// Gets the appropriate [`ContextBuilder`] for the given symbol.
pub async fn get_context_builder(
    accounts_cache: Arc<AccountsCache>,
    shutdown_sender: Arc<Sender<bool>>,
    context_info: &ContextInfo,
    _user_info: &UserInfo,
) -> Result<Arc<dyn ContextBuilder<Output = OperationContext> + Send>, Error> {
    info!("Preparing Context Builder for {}", context_info.symbol);

    let context_builder: Arc<dyn ContextBuilder<Output = OperationContext> + Send> =
        match &context_info.context_accounts {
            Accounts::Futures(f) => Arc::new(DerivativeContextBuilder::<FuturesMarket>::new(
                accounts_cache,
                shutdown_sender,
                f.state,
                f.market,
                f.event_queue,
                f.bids,
                f.asks,
                f.orders,
                f.quote_pool,
                f.quote_pool_nodes.to_vec(),
                context_info.symbol.to_string(),
            )),
            Accounts::Perpetuals(p) => Arc::new(DerivativeContextBuilder::<PerpetualMarket>::new(
                accounts_cache,
                shutdown_sender,
                p.state,
                p.market,
                p.event_queue,
                p.bids,
                p.asks,
                p.orders,
                p.quote_pool,
                p.quote_pool_nodes.to_vec(),
                context_info.symbol.to_string(),
            )),
            Accounts::Spot(s) => Arc::new(SpotContextBuilder::new(
                accounts_cache,
                shutdown_sender,
                s.state,
                s.market,
                s.event_queue,
                s.bids,
                s.asks,
                s.open_orders,
                s.asset_pool,
                s.asset_pool_nodes.to_vec(),
                s.quote_pool,
                s.quote_pool_nodes.to_vec(),
                context_info.symbol.to_string(),
            )),
        };

    Ok(context_builder)
}

/// Gets the appropriate [`ContextManager`] for the given config.
pub fn get_context_manager_from_config(
    _ctx: &CypherContext,
    config: &Config<MarketMakerConfig>,
    shutdown_sender: Arc<Sender<bool>>,
    global_context_builder: Arc<dyn ContextBuilder<Output = GlobalContext> + Send>,
    operation_context_builder: Arc<dyn ContextBuilder<Output = OperationContext> + Send>,
    oracle_provider: Arc<dyn OracleProvider<Input = GlobalContext> + Send>,
) -> Result<
    Arc<
        dyn ContextManager<
            Output = ExecutionContext,
            GlobalContextInput = GlobalContext,
            OperationContextInput = OperationContext,
            OracleInfoInput = OracleInfo,
        >,
    >,
    Error,
> {
    let symbol = &config.inner.hedger_config.symbol;
    info!("Preparing Context Manager for {}", symbol);

    let context_manager: Arc<
        dyn ContextManager<
            Output = ExecutionContext,
            GlobalContextInput = GlobalContext,
            OperationContextInput = OperationContext,
            OracleInfoInput = OracleInfo,
        >,
    > = Arc::new(CypherExecutionContextManager::new(
        shutdown_sender,
        global_context_builder.clone(),
        operation_context_builder.clone(),
        oracle_provider.clone(),
    ));
    Ok(context_manager)
}

/// Gets an [`OracleProvider`] for the given [`ContextInfo`] and config.
pub async fn get_oracle_provider(
    context_info: &ContextInfo,
    shutdown_sender: Arc<Sender<bool>>,
    global_context_builder: Arc<dyn ContextBuilder<Output = GlobalContext> + Send>,
) -> Result<Arc<dyn OracleProvider<Input = GlobalContext> + Send>, Error> {
    let oracle_provider: Arc<dyn OracleProvider<Input = GlobalContext> + Send> =
        Arc::new(CypherOracleProvider::new(
            global_context_builder.clone(),
            shutdown_sender,
            context_info.market_metadata.cache_index as usize,
            context_info.symbol.to_string(),
        ));

    Ok(oracle_provider)
}

/// Gets the decimals of a given asset or derivatives market position by it's symbol
async fn get_decimals_for_symbol(ctx: &CypherContext, symbol: &str) -> Result<u8, Error> {
    if symbol.contains("-PERP") {
        match get_perp_market_from_symbol(ctx, symbol).await {
            Ok(ctx) => Ok(ctx.state.inner.config.decimals),
            Err(e) => Err(e),
        }
    } else if symbol.contains("1!") {
        match get_futures_market_from_symbol(ctx, symbol).await {
            Ok(ctx) => Ok(ctx.state.inner.config.decimals),
            Err(e) => Err(e),
        }
    } else {
        match get_pool_from_symbol(ctx, symbol).await {
            Ok(ctx) => Ok(ctx.state.config.decimals),
            Err(e) => Err(e),
        }
    }
}

/// Gets the [`PoolContext`] for the given symbol.
async fn get_pool_from_symbol(ctx: &CypherContext, symbol: &str) -> Result<PoolContext, Error> {
    let pools = ctx.pools.read().await;

    for pool in pools.iter() {
        let market_name = from_utf8(&pool.state.pool_name)
            .unwrap()
            .trim_matches(char::from(0));
        if market_name == symbol {
            return Ok(pool.to_owned());
        }
    }

    Err(Error::InvalidConfig(ConfigError::UnrecognizedSymbol(
        symbol.to_string(),
    )))
}

/// Gets the [`SpotMarketContext`] for the given symbol.
async fn get_spot_market_from_symbol(
    ctx: &CypherContext,
    symbol: &str,
) -> Result<SpotMarketContext, Error> {
    let spot_markets = ctx.spot_markets.read().await;
    let pools = ctx.pools.read().await;

    for pool in pools.iter() {
        let market_name = from_utf8(&pool.state.pool_name)
            .unwrap()
            .trim_matches(char::from(0));
        if market_name == symbol {
            let spot_market = spot_markets
                .iter()
                .find(|s| s.address == pool.state.dex_market)
                .unwrap();
            return Ok(spot_market.to_owned());
        }
    }

    Err(Error::InvalidConfig(ConfigError::UnrecognizedSymbol(
        symbol.to_string(),
    )))
}

/// Gets the [`SpotMarketContext`] and [`PoolContext`] for the given symbol.
async fn get_spot_market_and_pool_from_symbol(
    ctx: &CypherContext,
    symbol: &str,
) -> Result<(SpotMarketContext, PoolContext), Error> {
    let spot_markets = ctx.spot_markets.read().await;
    let pools = ctx.pools.read().await;

    for pool in pools.iter() {
        let market_name = from_utf8(&pool.state.pool_name)
            .unwrap()
            .trim_matches(char::from(0));
        if market_name == symbol {
            let spot_market = spot_markets
                .iter()
                .find(|s| s.address == pool.state.dex_market)
                .unwrap();
            return Ok((spot_market.to_owned(), pool.to_owned()));
        }
    }

    Err(Error::InvalidConfig(ConfigError::UnrecognizedSymbol(
        symbol.to_string(),
    )))
}

/// Gets the [`MarketContext<PerpetualMarket>`] for the given symbol.
async fn get_perp_market_from_symbol(
    ctx: &CypherContext,
    symbol: &str,
) -> Result<MarketContext<PerpetualMarket>, Error> {
    let perp_markets = ctx.perp_markets.read().await;

    for perp_market in perp_markets.iter() {
        let market_name = from_utf8(&perp_market.state.inner.market_name)
            .unwrap()
            .trim_matches(char::from(0));
        if market_name == symbol {
            return Ok(perp_market.clone());
        }
    }

    Err(Error::InvalidConfig(ConfigError::UnrecognizedSymbol(
        symbol.to_string(),
    )))
}

/// Gets the [`MarketContext<FuturesMarket>`] for the given symbol.
async fn get_futures_market_from_symbol(
    ctx: &CypherContext,
    symbol: &str,
) -> Result<MarketContext<FuturesMarket>, Error> {
    let futures_markets = ctx.futures_markets.read().await;

    for futures_market in futures_markets.iter() {
        let market_name = from_utf8(&futures_market.state.inner.market_name)
            .unwrap()
            .trim_matches(char::from(0));
        if market_name == symbol {
            return Ok(futures_market.clone());
        }
    }

    Err(Error::InvalidConfig(ConfigError::UnrecognizedSymbol(
        symbol.to_string(),
    )))
}
