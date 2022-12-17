use clap::{App, Arg, ArgMatches, SubCommand};
use cypher_client::{
    cache_account,
    constants::QUOTE_TOKEN_DECIMALS,
    instructions::{
        new_spot_order as new_spot_order_ix, update_account_margin as update_account_margin_ix,
    },
    utils::{
        convert_coin_to_lots, convert_price_to_lots, derive_account_address,
        derive_orders_account_address, derive_pool_address, derive_pool_node_address,
        derive_pool_node_vault_signer_address, derive_public_clearing_address,
        derive_spot_open_orders_address, derive_sub_account_address, fixed_to_ui,
        fixed_to_ui_price, gen_dex_vault_signer_key, convert_price_to_decimals, convert_coin_to_decimals, convert_pc_to_decimals,
    },
    Clearing, CypherAccount, NewSpotOrderArgs, OrderType, SelfTradeBehavior, Side,
};
use cypher_utils::{
    contexts::CypherContext,
    utils::{encode_string, get_cypher_zero_copy_account, send_transactions},
};
use fixed::types::I80F48;
use solana_sdk::{pubkey::Pubkey, signer::Signer};
use std::{
    error,
    ops::Mul,
    str::{from_utf8, FromStr},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{cli::CliError, utils::accounts::get_or_create_spot_orders_account};

use super::{command::CliCommand, CliConfig, CliResult};

#[derive(Debug)]
pub enum SpotSubCommand {
    Orders,
    Market {
        symbol: String,
        side: Side,
        size: I80F48,
    },
    Place {
        symbol: String,
        side: Side,
        size: I80F48,
        price: I80F48,
        order_type: String,
    },
}

pub trait SpotSubCommands {
    fn spot_subcommands(self) -> Self;
}

impl SpotSubCommands for App<'_, '_> {
    fn spot_subcommands(self) -> Self {
        self.subcommand(
            SubCommand::with_name("spot")
                .about("Subcommands that interact with Spot Markets.")
                .subcommand(
                    SubCommand::with_name("orders")
                        .about("Lists all existing open orders by market."),
                )
                .subcommand(
                    SubCommand::with_name("market")
                        .about("Market buys or sells a certain size.")
                        .arg(
                            Arg::with_name("symbol")
                                .short("s")
                                .long("symbol")
                                .takes_value(true)
                                .help("The market symbol, e.g. \"SOL\"."),
                        )
                        .arg(
                            Arg::with_name("side")
                                .long("side")
                                .takes_value(true)
                                .help("The side of the order, must be one of \"buy\" or \"sell\".")
                        )
                        .arg(
                            Arg::with_name("size")
                                .long("size")
                                .takes_value(true)
                                .help("The size of the order, value should be in base token, non-native units.")
                        ),
                )
                .subcommand(
                    SubCommand::with_name("place")
                        .about("Places an order of given size at given price.")
                        .arg(
                            Arg::with_name("symbol")
                                .short("s")
                                .long("symbol")
                                .takes_value(true)
                                .help("The market symbol, e.g. \"SOL\"."),
                        )
                        .arg(
                            Arg::with_name("side")
                                .long("side")
                                .takes_value(true)
                                .help("The side of the order, must be one of \"buy\" or \"sell\".")
                        )
                        .arg(
                            Arg::with_name("size")
                                .long("size")
                                .takes_value(true)
                                .help("The size of the order, value should be in base token, non-native units.")
                        )
                        .arg(
                            Arg::with_name("price")
                                .long("price")
                                .takes_value(true)
                                .help("The price of the order, value should be in quote token, non-native units.")
                        )
                        .arg(
                            Arg::with_name("order-type")
                                .long("order-type")
                                .takes_value(true)
                                .help("The order type, must be one of \"limit\" or \"postOnly\".")
                        ),
                ),
        )
    }
}

pub fn parse_spot_command(matches: &ArgMatches) -> Result<CliCommand, Box<dyn error::Error>> {
    match matches.subcommand() {
        ("orders", Some(_matches)) => Ok(CliCommand::Spot(SpotSubCommand::Orders)),
        ("market", Some(matches)) => {
            // market symbol
            let symbol = match matches.value_of("symbol") {
                Some(s) => s,
                None => {
                    return Err(Box::new(CliError::BadParameters(
                        "Symbol not provided, to see available markets try \"list pools\" command."
                            .to_string(),
                    )));
                }
            };
            // order side
            let side = match matches.value_of("side") {
                Some(s) => match s {
                    "buy" => Side::Bid,
                    "sell" => Side::Ask,
                    _ => {
                        return Err(Box::new(CliError::BadParameters(
                            "Invalid order side provided, must be one of \"buy\" or \"sell\"."
                                .to_string(),
                        )));
                    }
                },
                None => {
                    return Err(Box::new(CliError::BadParameters(
                        "Order side not provided, must be one of \"buy\" or \"sell\".".to_string(),
                    )));
                }
            };
            // order size
            let size = match matches.value_of("size") {
                Some(s) => match I80F48::from_str(s) {
                    Ok(s) => s,
                    Err(e) => {
                        return Err(Box::new(CliError::BadParameters(format!(
                            "Invalid order size.: {}",
                            e.to_string()
                        ))));
                    }
                },
                None => {
                    return Err(Box::new(CliError::BadParameters(
                        "Order size not provided. value should be in base token, non-native units."
                            .to_string(),
                    )));
                }
            };
            Ok(CliCommand::Spot(SpotSubCommand::Market {
                symbol: symbol.to_string(),
                side,
                size,
            }))
        }
        ("place", Some(matches)) => {
            // market symbol
            let symbol = match matches.value_of("symbol") {
                Some(s) => s,
                None => {
                    return Err(Box::new(CliError::BadParameters(
                        "Symbol not provided. To see available markets try \"list pools\" command."
                            .to_string(),
                    )));
                }
            };
            // order side
            let side = match matches.value_of("side") {
                Some(s) => match s {
                    "buy" => Side::Bid,
                    "sell" => Side::Ask,
                    _ => {
                        return Err(Box::new(CliError::BadParameters(
                            "Invalid order side provided, must be one of \"buy\" or \"sell\"."
                                .to_string(),
                        )));
                    }
                },
                None => {
                    return Err(Box::new(CliError::BadParameters(
                        "Order side not provided. Must be one of \"buy\" or \"sell\".".to_string(),
                    )));
                }
            };
            // order size
            let size = match matches.value_of("size") {
                Some(s) => match I80F48::from_str(s) {
                    Ok(s) => s,
                    Err(e) => {
                        return Err(Box::new(CliError::BadParameters(format!(
                            "Invalid order size.: {}",
                            e.to_string()
                        ))));
                    }
                },
                None => {
                    return Err(Box::new(CliError::BadParameters(
                        "Order size not provided. Value should be in base token, non-native units."
                            .to_string(),
                    )));
                }
            };
            // order price
            let price = match matches.value_of("price") {
                Some(s) => match I80F48::from_str(s) {
                    Ok(s) => s,
                    Err(e) => {
                        return Err(Box::new(CliError::BadParameters(format!(
                            "Invalid price: {}",
                            e.to_string()
                        ))));
                    }
                },
                None => {
                    return Err(Box::new(CliError::BadParameters(
                        "Price not provided. Value should be in quote token, non-native units."
                            .to_string(),
                    )));
                }
            };
            let order_type = match matches.value_of("order-type") {
                Some(s) => s.to_string(),
                None => {
                    return Err(Box::new(CliError::BadParameters(
                        "Order type not provided. Must be one of \"limit\" or \"postOnly\"."
                            .to_string(),
                    )));
                }
            };
            Ok(CliCommand::Spot(SpotSubCommand::Place {
                symbol: symbol.to_string(),
                side,
                size,
                price,
                order_type,
            }))
        }
        ("", None) => {
            eprintln!("{}", matches.usage());
            Err(Box::new(CliError::CommandNotRecognized(
                "No market maker subcommand given.".to_string(),
            )))
        }
        _ => unreachable!(),
    }
}

pub async fn list_spot_open_orders(config: &CliConfig) -> Result<CliResult, Box<dyn error::Error>> {
    Ok(CliResult {})
}

pub async fn process_spot_market_order(
    config: &CliConfig,
    symbol: &str,
    side: Side,
    size: I80F48,
) -> Result<CliResult, Box<dyn error::Error>> {
    Ok(CliResult {})
}

pub async fn process_spot_limit_order(
    config: &CliConfig,
    symbol: &str,
    side: Side,
    size: I80F48,
    price: I80F48,
    order_type: &str,
) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();
    let keypair = config.keypair.as_ref().unwrap();

    let (public_clearing, _) = derive_public_clearing_address();

    let clearing =
        match get_cypher_zero_copy_account::<Clearing>(rpc_client, &public_clearing).await {
            Ok(ctx) => ctx,
            Err(e) => {
                eprintln!("Failed to load Clearing.");
                return Err(Box::new(CliError::ClientError(e)));
            }
        };

    let ctx = match CypherContext::load(rpc_client).await {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Failed to load Cypher Context.");
            return Err(Box::new(CliError::ContextError(e)));
        }
    };

    let pools = ctx.pools.read().await;
    let asset_pool = pools
        .iter()
        .find(|p| from_utf8(&p.state.pool_name).unwrap().trim_matches('\0') == symbol)
        .unwrap();
    let asset_pool_node = asset_pool.pool_nodes.first().unwrap(); // TODO: change this
    let pool_name = from_utf8(&asset_pool.state.pool_name)
        .unwrap()
        .trim_matches(char::from(0));
    let quote_pool = pools
        .iter()
        .find(|p| from_utf8(&p.state.pool_name).unwrap().trim_matches('\0') == "USDC".to_string())
        .unwrap();
    let quote_pool_node = quote_pool.pool_nodes.first().unwrap(); // TODO: change this

    let spot_markets = ctx.spot_markets.read().await;
    let market_ctx = spot_markets
        .iter()
        .find(|m| m.address == asset_pool.state.dex_market)
        .unwrap();

    let order_type = if order_type == "limit" {
        OrderType::Limit
    } else {
        OrderType::PostOnly
    };

    let limit_price = convert_price_to_lots(
        price
            .mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32)))
            .to_num(),
        market_ctx.state.coin_lot_size,
        10u64.pow(asset_pool.state.config.decimals as u32),
        market_ctx.state.pc_lot_size,
    );

    let (master_account, _) = derive_account_address(&keypair.pubkey(), 0); // TODO: change this, allow multiple accounts
    let (sub_account, _) = derive_sub_account_address(&master_account, 0); // TODO: change this, allow multiple accounts
    let (orders_account, _) = derive_spot_open_orders_address(
        &asset_pool.state.dex_market,
        &master_account,
        &sub_account,
    );

    let account = get_cypher_zero_copy_account::<CypherAccount>(&rpc_client, &master_account)
        .await
        .unwrap();

    let sub_accounts = account
        .sub_account_caches
        .iter()
        .filter(|c| c.sub_account != Pubkey::default())
        .map(|c| c.sub_account)
        .collect::<Vec<Pubkey>>();

    let orders_account_state = match get_or_create_spot_orders_account(
        &rpc_client,
        keypair,
        &master_account,
        &sub_account,
        &asset_pool.state.dex_market,
        &asset_pool.address,
        &asset_pool.state.token_mint,
        &orders_account,
    )
    .await
    {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Failed to get or create Orders Account.");
            return Err(Box::new(CliError::ClientError(e)));
        }
    };

    let user_fee_tier = clearing.get_fee_tier(account.fee_tier);
    println!(
        "(debug) User Fee Tier: {} | Maker: {} | Taker: {} | Rebate: {}",
        user_fee_tier.tier,
        user_fee_tier.maker_bps,
        user_fee_tier.taker_bps,
        user_fee_tier.rebate_bps
    );

    let max_coin_qty = size
        .mul(I80F48::from(
            10u64.pow(asset_pool.state.config.decimals as u32),
        ))
        .to_num();
    let max_coin_qty = convert_coin_to_lots(max_coin_qty, market_ctx.state.coin_lot_size);

    let max_native_pc_qty_including_fees = if order_type == OrderType::PostOnly {
        max_coin_qty * limit_price
    } else {
        let max_quote_qty_without_fee = max_coin_qty * limit_price;
        (max_quote_qty_without_fee * (10_001 + user_fee_tier.taker_bps as u64)) / 10_000
    };

    println!(
        "(debug) Price: {} | Size: {} | Notional: {}",
        limit_price, max_coin_qty, max_native_pc_qty_including_fees
    );
    println!(
        "Placing limit order on {} at price {:.5} with size {:.5} for total quote quantity of {:.5}.",
        pool_name,
        fixed_to_ui(
            I80F48::from(
                convert_price_to_decimals(
                    limit_price,
                    market_ctx.state.coin_lot_size, 
                    10u64.pow(asset_pool.state.config.decimals as u32),
                    market_ctx.state.pc_lot_size)
                ),
            QUOTE_TOKEN_DECIMALS
        ),
        fixed_to_ui(
            I80F48::from(convert_coin_to_decimals(max_coin_qty, market_ctx.state.coin_lot_size)),
            asset_pool.state.config.decimals
        ),
        fixed_to_ui(
            I80F48::from(convert_pc_to_decimals(max_native_pc_qty_including_fees, market_ctx.state.pc_lot_size)),
            QUOTE_TOKEN_DECIMALS
        )
    );

    let vault_signer = if side == Side::Bid {
        derive_pool_node_vault_signer_address(&quote_pool_node.address).0
    } else {
        derive_pool_node_vault_signer_address(&asset_pool_node.address).0
    };

    let dex_vault_signer =
        gen_dex_vault_signer_key(market_ctx.state.vault_signer_nonce, &market_ctx.address).unwrap();

    let args = NewSpotOrderArgs {
        side,
        limit_price,
        max_coin_qty,
        max_native_pc_qty_including_fees,
        order_type,
        self_trade_behavior: SelfTradeBehavior::AbortTransaction,
        client_order_id: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        limit: u16::MAX,
    };
    let ixs = vec![
        update_account_margin_ix(
            &cache_account::id(),
            &master_account,
            &keypair.pubkey(),
            &sub_accounts,
        ),
        new_spot_order_ix(
            &public_clearing,
            &cache_account::id(),
            &master_account,
            &sub_account,
            &asset_pool_node.address,
            &quote_pool_node.address,
            &asset_pool.state.token_mint,
            &asset_pool_node.state.token_vault,
            &quote_pool_node.state.token_vault,
            &vault_signer,
            &keypair.pubkey(),
            &market_ctx.address,
            &orders_account,
            &market_ctx.event_queue,
            &market_ctx.request_queue,
            &market_ctx.bids,
            &market_ctx.asks,
            &market_ctx.base_vault,
            &market_ctx.quote_vault,
            &dex_vault_signer,
            args,
        ),
    ];

    let sig = match send_transactions(&rpc_client, ixs, keypair, true).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to submit transaction.");
            return Err(Box::new(CliError::ClientError(e)));
        }
    };

    println!(
        "Successfully submitted order. Transaction signtaure: {}",
        sig.first().unwrap()
    );

    Ok(CliResult {})
}
