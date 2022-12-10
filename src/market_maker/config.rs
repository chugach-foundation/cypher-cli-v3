use cypher_client::{
    utils::{derive_orders_account_address, derive_spot_open_orders_address},
    FuturesMarket, PerpetualMarket,
};
use cypher_utils::{
    accounts_cache::AccountsCache,
    contexts::{CypherContext, MarketContext, PoolContext, SpotMarketContext},
    services::StreamingAccountInfoService,
    utils::get_dex_account,
};
use fixed::types::I80F48;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use serde_json;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{pubkey::Pubkey, signature::Keypair};
use std::{error, fs::File, io::BufReader, str::from_utf8, sync::Arc};
use thiserror::Error;

use crate::{
    common::{
        context::{
            builder::ContextBuilder, manager::ContextManager, GlobalContext, OperationContext,
        },
        hedger::Hedger,
        maker::Maker,
        orders::OrderManager,
    },
    context::{
        builders::{
            derivatives::DerivativeContextBuilder, global::GlobalContextBuilder,
            spot::SpotContextBuilder,
        },
        cypher_manager::CypherExecutionContextManager,
    },
    market_maker::{
        constants::BPS_UNIT,
        hedging::{futures::FuturesHedger, perps::PerpsHedger, spot::SpotHedger},
        inventory::InventoryManager,
        making::{futures::FuturesMaker, perps::PerpsMaker, spot::SpotMaker},
        orders::{futures::FuturesOrderManager, perps::PerpsOrderManager, spot::SpotOrderManager},
    },
    utils::accounts::{get_or_create_orders_account, get_or_create_spot_orders_account},
};

use super::error::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Unrecognized symbol: {0}")]
    UnrecognizedSymbol(String),
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub account_number: u8,
    pub sub_account_number: u8,
    pub maker_config: MakerConfig,
    pub hedger_config: HedgerConfig,
    pub inventory_config: InventoryConfig,
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
    pub spacing_bps: u32,
    /// the amount of units to increment between order layers
    pub step_amount: f64,
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

pub fn load_config(path: &str) -> Result<Config, Box<dyn error::Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mm_config: Config = serde_json::from_reader(reader).unwrap();
    Ok(mm_config)
}

/// Gets the appropriate [`Maker`] for the given config.
pub async fn get_maker_from_config(
    ctx: &CypherContext,
    config: &Config,
) -> Result<Arc<dyn Maker>, Error> {
    let symbol = &config.maker_config.symbol;
    info!("Preparing Maker for {}.", symbol);

    info!(
        "Inventory management - Max quote: {} - Target Spread: {} bps - Layers: {} - Step: {} bps - Spacing: {} bps",
        config.maker_config.max_quote,
        config.maker_config.spread,
        config.maker_config.layers,
        config.maker_config.step_amount,
        config.maker_config.spacing_bps
    );
    let decimals = get_decimals_for_symbol(ctx, symbol.as_str()).await?;
    let inventory_mngr = InventoryManager::new(
        decimals,
        config.inventory_config.exp_base,
        I80F48::from_num::<f64>(config.maker_config.max_quote),
        I80F48::from(config.inventory_config.shape_numerator),
        I80F48::from(config.inventory_config.shape_denominator),
        I80F48::from(BPS_UNIT)
            .checked_add(I80F48::from(config.maker_config.spread))
            .and_then(|n| n.checked_div(I80F48::from(BPS_UNIT)))
            .unwrap(),
    );

    let maker: Arc<dyn Maker> = if symbol.contains("-PERP") {
        Arc::new(PerpsMaker::new(symbol.to_string(), inventory_mngr))
    } else if symbol.contains("1!") {
        Arc::new(FuturesMaker::new(symbol.to_string(), inventory_mngr))
    } else {
        Arc::new(SpotMaker::new(symbol.to_string(), inventory_mngr))
    };

    Ok(maker)
}

/// Gets the appropriate [`Hedger`] for the given config.
pub fn get_hedger_from_config(
    ctx: &CypherContext,
    config: &Config,
) -> Result<Arc<dyn Hedger>, Error> {
    let symbol = &config.hedger_config.symbol;
    info!("Preparing Hedger for {}", symbol);

    let hedger: Arc<dyn Hedger> = if symbol.contains("-PERP") {
        Arc::new(PerpsHedger::new(symbol.to_string()))
    } else if symbol.contains("1!") {
        Arc::new(FuturesHedger::new(symbol.to_string()))
    } else {
        Arc::new(SpotHedger::new(symbol.to_string()))
    };

    Ok(hedger)
}

/// Gets the appropriate maker [`ContextBuilder`] for the given config.
pub async fn get_maker_context_builder(
    streaming_account_info: &Arc<StreamingAccountInfoService>,
    rpc_client: &Arc<RpcClient>,
    ctx: &CypherContext,
    config: &Config,
    authority: &Keypair,
    accounts_cache: Arc<AccountsCache>,
    master_account: &Pubkey,
    sub_account: &Pubkey,
) -> Result<Arc<dyn ContextBuilder<Output = OperationContext> + Send>, Error> {
    let symbol = &config.maker_config.symbol;
    info!("Preparing Maker Context Builder for {}", symbol);

    let context_builder: Arc<dyn ContextBuilder<Output = OperationContext> + Send> = if symbol
        .contains("-PERP")
    {
        // get perp market from symbol
        let market_ctx = match get_perp_market_from_symbol(ctx, symbol).await {
            Ok(ctx) => ctx,
            Err(e) => {
                return Err(e);
            }
        };
        // get or create deriv orders account
        let orders_account = derive_orders_account_address(&market_ctx.address, &master_account).0;
        match get_or_create_orders_account(
            &rpc_client,
            authority,
            master_account,
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
        streaming_account_info
            .add_subscriptions(&vec![
                market_ctx.address,
                market_ctx.state.inner.event_queue,
                market_ctx.state.inner.bids,
                market_ctx.state.inner.asks,
                orders_account,
            ])
            .await;
        // instantiate deriv context builder for perps
        Arc::new(DerivativeContextBuilder::<PerpetualMarket>::new(
            accounts_cache.clone(),
            market_ctx.state.as_ref().clone(),
            market_ctx.address,
            market_ctx.state.inner.event_queue,
            market_ctx.state.inner.bids,
            market_ctx.state.inner.asks,
            orders_account,
            symbol.to_string(),
        ))
    } else if symbol.contains("1!") {
        // get futures market from symbol
        let market_ctx = match get_futures_market_from_symbol(ctx, symbol).await {
            Ok(ctx) => ctx,
            Err(e) => {
                return Err(e);
            }
        };
        // get or create deriv orders account
        let orders_account = derive_orders_account_address(&market_ctx.address, &master_account).0;
        match get_or_create_orders_account(
            &rpc_client,
            authority,
            master_account,
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
        streaming_account_info
            .add_subscriptions(&vec![
                market_ctx.address,
                market_ctx.state.inner.event_queue,
                market_ctx.state.inner.bids,
                market_ctx.state.inner.asks,
                orders_account,
            ])
            .await;
        // instantiate deriv context builder for futures
        Arc::new(DerivativeContextBuilder::<FuturesMarket>::new(
            accounts_cache.clone(),
            market_ctx.state.as_ref().clone(),
            market_ctx.address,
            market_ctx.state.inner.event_queue,
            market_ctx.state.inner.bids,
            market_ctx.state.inner.asks,
            orders_account,
            symbol.to_string(),
        ))
    } else {
        // get spot market from symbol
        let (spot_market, pool) = match get_spot_market_and_pool_from_symbol(ctx, symbol).await {
            Ok(ctx) => ctx,
            Err(e) => {
                return Err(e);
            }
        };
        // get or create spot open orders account
        let orders_account =
            derive_spot_open_orders_address(&spot_market.address, &master_account, &sub_account).0;
        match get_or_create_spot_orders_account(
            &rpc_client,
            authority,
            master_account,
            sub_account,
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
        streaming_account_info
            .add_subscriptions(&vec![
                spot_market.address,
                spot_market.event_queue,
                spot_market.bids,
                spot_market.asks,
                orders_account,
            ])
            .await;
        // instantiate spot context builder
        Arc::new(SpotContextBuilder::new(
            accounts_cache.clone(),
            spot_market.state,
            spot_market.address,
            spot_market.event_queue,
            spot_market.bids,
            spot_market.asks,
            orders_account,
            symbol.to_string(),
        ))
    };

    Ok(context_builder)
}

/// Gets the appropriate hedger [`ContextBuilder`] for the given config.
pub async fn get_hedger_context_builder(
    streaming_account_info: &Arc<StreamingAccountInfoService>,
    rpc_client: &Arc<RpcClient>,
    ctx: &CypherContext,
    config: &Config,
    authority: &Keypair,
    accounts_cache: Arc<AccountsCache>,
    master_account: &Pubkey,
    sub_account: &Pubkey,
) -> Result<Arc<dyn ContextBuilder<Output = OperationContext> + Send>, Error> {
    let symbol = &config.hedger_config.symbol;
    info!("Preparing Hedger Context Builder for {}", symbol);

    let context_builder: Arc<dyn ContextBuilder<Output = OperationContext> + Send> = if symbol
        .contains("-PERP")
    {
        // get perp market from symbol
        let market_ctx = match get_perp_market_from_symbol(ctx, symbol).await {
            Ok(ctx) => ctx,
            Err(e) => {
                return Err(e);
            }
        };
        // get or create derivs orders account
        let orders_account = derive_orders_account_address(&market_ctx.address, &master_account).0;
        match get_or_create_orders_account(
            &rpc_client,
            authority,
            master_account,
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
        streaming_account_info
            .add_subscriptions(&vec![
                market_ctx.address,
                market_ctx.state.inner.event_queue,
                market_ctx.state.inner.bids,
                market_ctx.state.inner.asks,
                orders_account,
            ])
            .await;
        // instantiate deriv context builder for perps
        Arc::new(DerivativeContextBuilder::<PerpetualMarket>::new(
            accounts_cache.clone(),
            market_ctx.state.as_ref().clone(),
            market_ctx.address,
            market_ctx.state.inner.event_queue,
            market_ctx.state.inner.bids,
            market_ctx.state.inner.asks,
            orders_account,
            symbol.to_string(),
        ))
    } else if symbol.contains("1!") {
        // get futures market from symbol
        let market_ctx = match get_futures_market_from_symbol(ctx, symbol).await {
            Ok(ctx) => ctx,
            Err(e) => {
                return Err(e);
            }
        };
        // get or create derivs orders account
        let orders_account = derive_orders_account_address(&market_ctx.address, &master_account).0;
        match get_or_create_orders_account(
            &rpc_client,
            authority,
            master_account,
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
        streaming_account_info
            .add_subscriptions(&vec![
                market_ctx.address,
                market_ctx.state.inner.event_queue,
                market_ctx.state.inner.bids,
                market_ctx.state.inner.asks,
                orders_account,
            ])
            .await;
        // instantiate deriv context builder for futures
        Arc::new(DerivativeContextBuilder::<FuturesMarket>::new(
            accounts_cache.clone(),
            market_ctx.state.as_ref().clone(),
            market_ctx.address,
            market_ctx.state.inner.event_queue,
            market_ctx.state.inner.bids,
            market_ctx.state.inner.asks,
            orders_account,
            symbol.to_string(),
        ))
    } else {
        // get spot market from symbol
        let (spot_market, pool) = match get_spot_market_and_pool_from_symbol(ctx, symbol).await {
            Ok(ctx) => ctx,
            Err(e) => {
                return Err(e);
            }
        };
        // get or create spot open orders account
        let orders_account =
            derive_spot_open_orders_address(&spot_market.address, &master_account, &sub_account).0;
        match get_or_create_spot_orders_account(
            &rpc_client,
            authority,
            master_account,
            sub_account,
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
        streaming_account_info
            .add_subscriptions(&vec![
                spot_market.address,
                spot_market.event_queue,
                spot_market.bids,
                spot_market.asks,
                orders_account,
            ])
            .await;
        // instantiate spot context builder
        Arc::new(SpotContextBuilder::new(
            accounts_cache.clone(),
            spot_market.state,
            spot_market.address,
            spot_market.event_queue,
            spot_market.bids,
            spot_market.asks,
            orders_account,
            symbol.to_string(),
        ))
    };

    Ok(context_builder)
}

/// Gets the appropriate [`ContextManager`] for the given config.
pub fn get_context_manager_from_config(
    ctx: &CypherContext,
    config: &Config,
    global_context_builder: Arc<dyn ContextBuilder<Output = GlobalContext> + Send>,
    operation_context_builder: Arc<dyn ContextBuilder<Output = OperationContext> + Send>,
) -> Result<Arc<dyn ContextManager>, Error> {
    let symbol = &config.hedger_config.symbol;
    info!("Preparing Context Manager for {}", symbol);

    let context_manager: Arc<dyn ContextManager> = Arc::new(CypherExecutionContextManager::new(
        global_context_builder.clone(),
        operation_context_builder.clone(),
    ));
    Ok(context_manager)
}

/// Gets the appropriate [`OrderManager`] for the given config.
pub fn get_order_manager_from_config(
    ctx: &CypherContext,
    config: &Config,
) -> Result<Arc<dyn OrderManager>, Error> {
    let symbol = &config.hedger_config.symbol;
    info!("Preparing Order Manager for {}", symbol);

    // todo: need to add a bunch of things here
    let order_manager: Arc<dyn OrderManager> = if symbol.contains("-PERP") {
        Arc::new(PerpsOrderManager::new())
    } else if symbol.contains("1!") {
        Arc::new(FuturesOrderManager::new())
    } else {
        Arc::new(SpotOrderManager::new())
    };

    Ok(order_manager)
}

/// Gets the decimals of a given asset or derivatives market position by it's symbol
async fn get_decimals_for_symbol(ctx: &CypherContext, symbol: &str) -> Result<u8, Error> {
    match get_perp_market_from_symbol(ctx, symbol).await {
        Ok(ctx) => return Ok(ctx.state.inner.config.decimals),
        Err(e) => {
            warn!("Could not find perp market for symbol: {}", symbol);
        }
    };

    match get_futures_market_from_symbol(ctx, symbol).await {
        Ok(ctx) => return Ok(ctx.state.inner.config.decimals),
        Err(e) => {
            warn!("Could not find futures market for symbol: {}", symbol);
        }
    };

    match get_pool_from_symbol(ctx, symbol).await {
        Ok(ctx) => return Ok(ctx.state.config.decimals),
        Err(e) => {
            warn!("Could not find pool for symbol: {}", symbol);
        }
    };

    Err(Error::InvalidConfig(ConfigError::UnrecognizedSymbol(
        symbol.to_string(),
    )))
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
