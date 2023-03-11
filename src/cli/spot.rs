use anchor_lang::AnchorSerialize;
use anchor_spl::dex::serum_dex::state::OpenOrders;
use bytemuck::bytes_of;
use clap::{App, Arg, ArgMatches, SubCommand};
use cypher_client::{
    cache_account,
    constants::QUOTE_TOKEN_DECIMALS,
    dex,
    instructions::{
        new_spot_order as new_spot_order_ix, settle_spot_funds,
        update_account_margin as update_account_margin_ix,
    },
    utils::{
        convert_coin_to_decimals, convert_coin_to_decimals_fixed, convert_coin_to_lots,
        convert_pc_to_decimals_fixed, convert_price_to_decimals, convert_price_to_decimals_fixed,
        convert_price_to_lots, derive_account_address, derive_pool_node_vault_signer_address,
        derive_public_clearing_address, derive_spot_open_orders_address,
        derive_sub_account_address, fixed_to_ui, gen_dex_vault_signer_key,
    },
    Clearing, CypherAccount, NewSpotOrderArgs, OrderType, SelfTradeBehavior, Side,
};
use cypher_utils::{
    contexts::{CypherContext, GenericOpenOrders, SerumOpenOrdersContext, SerumOrderBookContext},
    utils::{get_cypher_zero_copy_account, get_program_accounts, send_transactions},
};
use fixed::types::I80F48;
use solana_client::rpc_filter::{Memcmp, MemcmpEncodedBytes, RpcFilterType};
use solana_sdk::{pubkey::Pubkey, signer::Signer};
use std::{
    error,
    ops::{Add, Mul},
    str::{from_utf8, FromStr},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{cli::CliError, utils::accounts::get_or_create_spot_orders_account};

use super::{command::CliCommand, orderbook::display_spot_orderbook, CliConfig, CliResult};

#[derive(Debug)]
pub enum SpotSubCommand {
    Orders {
        pubkey: Option<Pubkey>,
    },
    Settle {
        symbol: String,
    },
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
    Book {
        symbol: String,
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
                        .about("Lists all existing open orders by market.")
                        .arg(
                            Arg::with_name("pubkey")
                                .long("pubkey")
                                .takes_value(true)
                                .help("The wallet pubkey."),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("settle")
                        .about("Settles funds by market.")
                        .arg(
                            Arg::with_name("symbol")
                                .short("s")
                                .long("symbol")
                                .takes_value(true)
                                .help("The market symbol, e.g. \"SOL\"."),
                        )
                )
                .subcommand(
                    SubCommand::with_name("book")
                        .about("Displays the book on a given market.")
                        .arg(
                            Arg::with_name("symbol")
                                .short("s")
                                .long("symbol")
                                .takes_value(true)
                                .help("The market symbol, e.g. \"SOL\"."),
                        )
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
        ("book", Some(matches)) => {
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
            Ok(CliCommand::Spot(SpotSubCommand::Book {
                symbol: symbol.to_string(),
            }))
        }
        ("settle", Some(matches)) => {
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
            Ok(CliCommand::Spot(SpotSubCommand::Settle {
                symbol: symbol.to_string(),
            }))
        }
        ("orders", Some(matches)) => {
            let pubkey = matches
                .value_of("pubkey")
                .map(|a| Pubkey::from_str(a).unwrap());
            Ok(CliCommand::Spot(SpotSubCommand::Orders { pubkey }))
        }
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
                            e
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
                            e
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
                            e
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

#[allow(deprecated)]
pub async fn list_spot_open_orders(
    config: &CliConfig,
    pubkey: Option<Pubkey>,
) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();
    let keypair = config.keypair.as_ref().unwrap();

    let authority = if pubkey.is_some() {
        pubkey.unwrap()
    } else {
        keypair.pubkey()
    };
    println!("Using Authority: {}", authority);

    let ctx = match CypherContext::load(rpc_client).await {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Failed to load Cypher Context.");
            return Err(Box::new(CliError::ContextError(e)));
        }
    };

    let filters = vec![
        RpcFilterType::DataSize(12 + std::mem::size_of::<OpenOrders>() as u64),
        RpcFilterType::Memcmp(Memcmp {
            offset: 45,
            bytes: MemcmpEncodedBytes::Base58(authority.to_string()),
            encoding: None,
        }),
    ];
    let orders_accounts = match get_program_accounts(rpc_client, filters, &dex::id()).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Failed to fetch Open Orders Accounts.");
            return Err(Box::new(CliError::ClientError(e)));
        }
    };

    if orders_accounts.is_empty() {
        println!("No Orders Accounts found.");
        return Ok(CliResult {});
    }

    let markets = ctx.spot_markets.read().await;
    let pools = ctx.pools.read().await;

    for (pubkey, account) in orders_accounts.iter() {
        println!("Orders Account: {}", pubkey);
        let orders_account = SerumOpenOrdersContext::from_account_data(pubkey, &account.data);
        let orders_account_market = orders_account.state.market;
        let market_pubkey = Pubkey::new(bytes_of(&orders_account_market));
        let market = match markets.iter().find(|m| m.address == market_pubkey) {
            Some(m) => m,
            None => {
                continue;
            }
        };
        let pool = match pools.iter().find(|p| p.state.dex_market == market_pubkey) {
            Some(m) => m,
            None => {
                continue;
            }
        };
        let pool_name = from_utf8(&pool.state.pool_name)
            .unwrap()
            .trim_matches(char::from(0));

        println!(
            "\n| {:^15} | {:^15} | {:^15} | {:^15} |",
            "B. Locked", "B. Total", "Q. Locked", "Q. Total",
        );

        if orders_account.state.native_coin_free != 0
            || orders_account.state.native_coin_total != 0
            || orders_account.state.native_pc_free != 0
            || orders_account.state.native_pc_total != 0
        {
            println!(
                "| {:>15.4} | {:>15.4} | {:>15.4} | {:>15.4} |",
                fixed_to_ui(
                    convert_coin_to_decimals_fixed(
                        orders_account.state.native_coin_total
                            - orders_account.state.native_coin_free,
                        market.state.coin_lot_size
                    ),
                    pool.state.config.decimals
                ),
                fixed_to_ui(
                    convert_coin_to_decimals_fixed(
                        orders_account.state.native_coin_total,
                        market.state.coin_lot_size
                    ),
                    pool.state.config.decimals
                ),
                fixed_to_ui(
                    convert_pc_to_decimals_fixed(
                        orders_account.state.native_pc_total - orders_account.state.native_pc_free,
                        market.state.pc_lot_size
                    ),
                    QUOTE_TOKEN_DECIMALS
                ),
                fixed_to_ui(
                    convert_pc_to_decimals_fixed(
                        orders_account.state.native_pc_total,
                        market.state.pc_lot_size
                    ),
                    QUOTE_TOKEN_DECIMALS
                ),
            );
        }

        let book = match SerumOrderBookContext::load(
            rpc_client,
            &market.state,
            &market.address,
            &market.bids,
            &market.asks,
        )
        .await
        {
            Ok(a) => a,
            Err(e) => {
                eprintln!("Failed to load Order Book Context.");
                return Err(Box::new(CliError::ContextError(e)));
            }
        };

        let open_orders = GenericOpenOrders::get_open_orders(&orders_account, &book);

        println!(
            "\n| {:^10} | {:^45} | {:^4} | {:^15} | {:^15} | {:^15} |",
            "Name", "Order ID", "Side", "Base Qty.", "Notional Size", "Price",
        );

        let mut bid_base_qty = I80F48::ZERO;
        let mut bid_quote_qty = I80F48::ZERO;
        let mut ask_base_qty = I80F48::ZERO;
        let mut ask_quote_qty = I80F48::ZERO;

        for order in open_orders.iter() {
            let quote_quantity = fixed_to_ui(
                convert_pc_to_decimals_fixed(order.quote_quantity, market.state.pc_lot_size),
                QUOTE_TOKEN_DECIMALS,
            );
            let base_quantity = fixed_to_ui(
                convert_coin_to_decimals_fixed(order.base_quantity, market.state.coin_lot_size),
                pool.state.config.decimals,
            );
            if order.side == Side::Bid {
                bid_base_qty = bid_base_qty.add(base_quantity);
                bid_quote_qty = bid_quote_qty.add(quote_quantity);
            } else {
                ask_base_qty = ask_base_qty.add(base_quantity);
                ask_quote_qty = ask_quote_qty.add(quote_quantity);
            }
            println!(
                "| {:^10} | {:^45} | {:<4} | {:>15.2} | {:>15.2} | {:>15.6} |",
                pool_name,
                order.order_id,
                order.side.to_string(),
                base_quantity,
                quote_quantity,
                fixed_to_ui(
                    convert_price_to_decimals_fixed(
                        order.price,
                        market.state.coin_lot_size,
                        10u64.pow(pool.state.config.decimals as u32),
                        market.state.pc_lot_size
                    ),
                    QUOTE_TOKEN_DECIMALS
                ),
            );
        }

        println!(
            "\n| {:^10} | {:^15} | {:^15} |",
            "Side", "Base Qty.", "Quote Qty."
        );

        println!(
            "| {:^10} | {:>15.4} | {:>15.4} |",
            "Buy", bid_base_qty, bid_quote_qty
        );
        println!(
            "| {:^10} | {:>15.4} | {:>15.4} |",
            "Sell", ask_base_qty, ask_quote_qty
        );
    }

    Ok(CliResult {})
}

pub async fn process_spot_market_order(
    config: &CliConfig,
    symbol: &str,
    side: Side,
    size: I80F48,
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
        .find(|p| from_utf8(&p.state.pool_name).unwrap().trim_matches('\0') == "USDC")
        .unwrap();
    let quote_pool_node = quote_pool.pool_nodes.first().unwrap(); // TODO: change this

    let spot_markets = ctx.spot_markets.read().await;
    let market_ctx = spot_markets
        .iter()
        .find(|m| m.address == asset_pool.state.dex_market)
        .unwrap();

    let (master_account, _) = derive_account_address(&keypair.pubkey(), 0); // TODO: change this, allow multiple accounts
    let (sub_account, _) = derive_sub_account_address(&master_account, 0); // TODO: change this, allow multiple accounts
    let (orders_account, _) = derive_spot_open_orders_address(
        &asset_pool.state.dex_market,
        &master_account,
        &sub_account,
    );

    let account = get_cypher_zero_copy_account::<CypherAccount>(rpc_client, &master_account)
        .await
        .unwrap();

    let sub_accounts = account
        .sub_account_caches
        .iter()
        .filter(|c| c.sub_account != Pubkey::default())
        .map(|c| c.sub_account)
        .collect::<Vec<Pubkey>>();

    let _ = match get_or_create_spot_orders_account(
        rpc_client,
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

    let ob_ctx = match SerumOrderBookContext::load(
        rpc_client,
        &market_ctx.state,
        &market_ctx.address,
        &market_ctx.bids,
        &market_ctx.asks,
    )
    .await
    {
        Ok(ob) => ob,
        Err(e) => {
            return Err(Box::new(CliError::ContextError(e)));
        }
    };
    let max_coin_qty = convert_coin_to_lots(
        size.mul(I80F48::from(
            10u64.pow(asset_pool.state.config.decimals as u32),
        ))
        .to_num(),
        market_ctx.state.coin_lot_size,
    );

    let impact_price = ob_ctx.get_impact_price(max_coin_qty, side);
    if impact_price.is_none() {
        return Err(Box::new(CliError::BadParameters(
            format!(
                "There is no available liquidity on the Order Book for Perp Market: {} to match size: {}",
                symbol,
                size
            )
        )));
    }

    let user_fee_tier = clearing.get_fee_tier(account.fee_tier);
    println!(
        "(debug) User Fee Tier: {} | Maker: {} | Taker: {} | Rebate: {}",
        user_fee_tier.tier,
        user_fee_tier.maker_bps,
        user_fee_tier.taker_bps,
        user_fee_tier.rebate_bps
    );

    let limit_price = impact_price.unwrap();

    let max_native_pc_qty_including_fees = {
        let max_quote_qty_without_fee = max_coin_qty * limit_price;
        (max_quote_qty_without_fee
            * market_ctx.state.pc_lot_size
            * (10_000 + user_fee_tier.taker_bps as u64))
            / 10_000
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
            I80F48::from(max_native_pc_qty_including_fees),
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
        order_type: OrderType::ImmediateOrCancel,
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

    let sig =
        match send_transactions(rpc_client, ixs, keypair, true, Some((1_400_000, 1)), None).await {
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
        .find(|p| from_utf8(&p.state.pool_name).unwrap().trim_matches('\0') == "USDC")
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

    let account = get_cypher_zero_copy_account::<CypherAccount>(rpc_client, &master_account)
        .await
        .unwrap();

    let sub_accounts = account
        .sub_account_caches
        .iter()
        .filter(|c| c.sub_account != Pubkey::default())
        .map(|c| c.sub_account)
        .collect::<Vec<Pubkey>>();

    let _orders_account_state = match get_or_create_spot_orders_account(
        rpc_client,
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
        max_coin_qty * limit_price * market_ctx.state.pc_lot_size
    } else {
        let max_quote_qty_without_fee = max_coin_qty * limit_price;
        (max_quote_qty_without_fee
            * market_ctx.state.pc_lot_size
            * (10_000 + user_fee_tier.taker_bps as u64))
            / 10_000
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
            I80F48::from(max_native_pc_qty_including_fees),
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

    let sig =
        match send_transactions(rpc_client, ixs, keypair, true, Some((1_400_000, 1)), None).await {
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

pub async fn process_spot_settle_funds(
    config: &CliConfig,
    symbol: &str,
) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();
    let keypair = config.keypair.as_ref().unwrap();

    let (public_clearing, _) = derive_public_clearing_address();

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

    let quote_pool = pools
        .iter()
        .find(|p| from_utf8(&p.state.pool_name).unwrap().trim_matches('\0') == "USDC")
        .unwrap();
    let quote_pool_node = quote_pool.pool_nodes.first().unwrap(); // TODO: change this

    let spot_markets = ctx.spot_markets.read().await;
    let market_ctx = spot_markets
        .iter()
        .find(|m| m.address == asset_pool.state.dex_market)
        .unwrap();

    let (master_account, _) = derive_account_address(&keypair.pubkey(), 0); // TODO: change this, allow multiple accounts
    let (sub_account, _) = derive_sub_account_address(&master_account, 0); // TODO: change this, allow multiple accounts
    let (orders_account, _) = derive_spot_open_orders_address(
        &asset_pool.state.dex_market,
        &master_account,
        &sub_account,
    );

    let account = get_cypher_zero_copy_account::<CypherAccount>(rpc_client, &master_account)
        .await
        .unwrap();

    let sub_accounts = account
        .sub_account_caches
        .iter()
        .filter(|c| c.sub_account != Pubkey::default())
        .map(|c| c.sub_account)
        .collect::<Vec<Pubkey>>();

    let dex_vault_signer =
        gen_dex_vault_signer_key(market_ctx.state.vault_signer_nonce, &market_ctx.address).unwrap();

    let ixs = vec![
        update_account_margin_ix(
            &cache_account::id(),
            &master_account,
            &keypair.pubkey(),
            &sub_accounts,
        ),
        settle_spot_funds(
            &public_clearing,
            &cache_account::id(),
            &master_account,
            &sub_account,
            &asset_pool_node.address,
            &quote_pool_node.address,
            &asset_pool.state.token_mint,
            &asset_pool_node.state.token_vault,
            &quote_pool_node.state.token_vault,
            &keypair.pubkey(),
            &market_ctx.address,
            &orders_account,
            &market_ctx.base_vault,
            &market_ctx.quote_vault,
            &dex_vault_signer,
        ),
    ];

    let sig =
        match send_transactions(rpc_client, ixs, keypair, true, Some((1_400_000, 1)), None).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to submit transaction.");
                return Err(Box::new(CliError::ClientError(e)));
            }
        };

    println!(
        "Successfully settled funds. Transaction signtaure: {}",
        sig.first().unwrap()
    );

    Ok(CliResult {})
}

pub async fn process_spot_book(
    config: &CliConfig,
    symbol: &str,
) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();

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

    let spot_markets = ctx.spot_markets.read().await;
    let market_ctx = spot_markets
        .iter()
        .find(|m| m.address == asset_pool.state.dex_market)
        .unwrap();

    let book_ctx = match SerumOrderBookContext::load(
        rpc_client,
        &market_ctx.state,
        &market_ctx.address,
        &market_ctx.bids,
        &market_ctx.asks,
    )
    .await
    {
        Ok(ob) => ob,
        Err(e) => {
            return Err(Box::new(CliError::ContextError(e)));
        }
    };

    display_spot_orderbook(
        &book_ctx,
        &market_ctx.state,
        asset_pool.state.config.decimals,
    );

    Ok(CliResult {})
}
