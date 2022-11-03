use clap::{App, Arg, ArgMatches, SubCommand};
use cypher_client::{
    cache_account,
    constants::QUOTE_TOKEN_DECIMALS,
    instructions::{cancel_perp_order, new_perp_order, settle_perp_funds},
    utils::{
        convert_price_to_lots, derive_account_address, derive_orders_account_address,
        derive_pool_address, derive_pool_node_address, derive_public_clearing_address,
        derive_sub_account_address, fixed_to_ui, fixed_to_ui_price, get_zero_copy_account,
    },
    CancelOrderArgs, DerivativeOrderType, NewDerivativeOrderArgs, OrdersAccount, SelfTradeBehavior,
    Side,
};
use cypher_utils::{
    contexts::{AgnosticOrderBookContext, CypherContext, UserContext},
    utils::{encode_string, get_program_accounts, send_transactions},
};
use fixed::types::I80F48;
use solana_client::rpc_filter::{Memcmp, MemcmpEncodedBytes, MemcmpEncoding, RpcFilterType};
use solana_sdk::signer::Signer;
use std::{
    error,
    ops::Mul,
    str::{from_utf8, FromStr},
};

use crate::{
    cli::{CliError, CliResult},
    utils::accounts::get_or_create_orders_account,
};

use super::{command::CliCommand, CliConfig};

#[derive(Debug)]
pub enum PerpetualsSubCommand {
    Orders,
    Cancel {
        symbol: String,
        order_id: u128,
        side: Side,
    },
    Close {
        symbol: String,
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
    },
}

pub trait PerpetualsSubCommands {
    fn perps_subcommands(self) -> Self;
}

impl PerpetualsSubCommands for App<'_, '_> {
    fn perps_subcommands(self) -> Self {
        self.subcommand(
            SubCommand::with_name("perps")
                .about("Subcommands that interact with Perp Markets.")
                .subcommand(
                    SubCommand::with_name("orders")
                        .about("Lists all existing open orders by market."),
                )
                .subcommand(
                    SubCommand::with_name("close")
                        .about("Closes an existing position a given market.")
                        .arg(
                            Arg::with_name("symbol")
                                .short("s")
                                .long("symbol")
                                .takes_value(true)
                                .help("The market symbol, e.g. \"SOL1!\"."),
                        )
                )
                .subcommand(
                    SubCommand::with_name("settle")
                        .about("Settles existing unsettled funds on a given market.")
                        .arg(
                            Arg::with_name("symbol")
                                .short("s")
                                .long("symbol")
                                .takes_value(true)
                                .help("The market symbol, e.g. \"SOL1!\"."),
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
                                .help("The market symbol, e.g. \"SOL-PERP\"."),
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
                        .about("Places a limit order of given size at given price.")
                        .arg(
                            Arg::with_name("symbol")
                                .short("s")
                                .long("symbol")
                                .takes_value(true)
                                .help("The market symbol, e.g. \"SOL-PERP\"."),
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
                        ),
                )
                .subcommand(
                    SubCommand::with_name("cancel")
                        .about("Cancels an order by ID.")
                        .arg(
                            Arg::with_name("symbol")
                                .short("s")
                                .long("symbol")
                                .takes_value(true)
                                .help("The market symbol, e.g. \"SOL1!\"."),
                        )
                        .arg(
                            Arg::with_name("side")
                                .long("side")
                                .takes_value(true)
                                .help("The side of the order, must be one of \"buy\" or \"sell\".")
                        )
                        .arg(
                            Arg::with_name("order-id")
                                .short("i")
                                .long("order-id")
                                .takes_value(true)
                                .help("The order ID, value should fit in a u128..")
                        ),
                )
        )
    }
}

pub fn parse_perps_command(matches: &ArgMatches) -> Result<CliCommand, Box<dyn error::Error>> {
    match matches.subcommand() {
        ("orders", Some(_matches)) => Ok(CliCommand::Perpetuals(PerpetualsSubCommand::Orders)),
        ("cancel", Some(matches)) => {
            // market symbol
            let symbol = match matches.value_of("symbol") {
                Some(s) => s,
                None => {
                    return Err(Box::new(CliError::BadParameters(
                        "Symbol not provided, to see available markets try \"list perps\" command."
                            .to_string(),
                    )));
                }
            };
            // order id
            let order_id = match matches.value_of("order-id") {
                Some(s) => match u128::from_str(s) {
                    Ok(oid) => oid,
                    Err(e) => {
                        return Err(Box::new(CliError::BadParameters(format!(
                            "Invalid Order ID: {}",
                            e.to_string()
                        ))));
                    }
                },
                None => {
                    return Err(Box::new(CliError::BadParameters(
                        "Order ID not provided, to see open orders try \"perps orders\" command."
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
            Ok(CliCommand::Perpetuals(PerpetualsSubCommand::Cancel {
                symbol: symbol.to_string(),
                order_id,
                side,
            }))
        }
        ("close", Some(matches)) => {
            // market symbol
            let symbol = match matches.value_of("symbol") {
                Some(s) => s,
                None => {
                    return Err(Box::new(CliError::BadParameters(
                        "Symbol not provided, to see available markets try \"list perps\" command."
                            .to_string(),
                    )));
                }
            };
            Ok(CliCommand::Perpetuals(PerpetualsSubCommand::Close {
                symbol: symbol.to_string(),
            }))
        }
        ("settle", Some(matches)) => {
            // market symbol
            let symbol = match matches.value_of("symbol") {
                Some(s) => s,
                None => {
                    return Err(Box::new(CliError::BadParameters(
                        "Symbol not provided, to see available markets try \"list perps\" command."
                            .to_string(),
                    )));
                }
            };
            Ok(CliCommand::Perpetuals(PerpetualsSubCommand::Settle {
                symbol: symbol.to_string(),
            }))
        }
        ("market", Some(matches)) => {
            // market symbol
            let symbol = match matches.value_of("symbol") {
                Some(s) => s,
                None => {
                    return Err(Box::new(CliError::BadParameters(
                        "Symbol not provided, to see available markets try \"list perps\" command."
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
            Ok(CliCommand::Perpetuals(PerpetualsSubCommand::Market {
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
                        "Symbol not provided. To see available markets try \"list perps\" command."
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
            Ok(CliCommand::Perpetuals(PerpetualsSubCommand::Place {
                symbol: symbol.to_string(),
                side,
                size,
                price,
            }))
        }
        ("", None) => {
            eprintln!("{}", matches.usage());
            Err(Box::new(CliError::CommandNotRecognized(
                "No perpetuals subcommand given.".to_string(),
            )))
        }
        _ => unreachable!(),
    }
}

pub async fn list_perps_open_orders(
    config: &CliConfig,
) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();
    let keypair = config.keypair.as_ref().unwrap();

    let ctx = match CypherContext::load(rpc_client).await {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Failed to load Cypher Context.");
            return Err(Box::new(CliError::ContextError(e)));
        }
    };
    let filters = vec![
        RpcFilterType::DataSize(8 + std::mem::size_of::<OrdersAccount>() as u64),
        RpcFilterType::Memcmp(Memcmp {
            offset: 8 + 8, // offset for authority pubkey on cypher's orders accounts, includes anchor discriminator
            bytes: MemcmpEncodedBytes::Base58(keypair.pubkey().to_string()),
            encoding: Some(MemcmpEncoding::Binary),
        }),
    ];
    let orders_accounts =
        match get_program_accounts(rpc_client, filters, &cypher_client::id()).await {
            Ok(a) => a,
            Err(e) => {
                eprintln!("Failed to fetch Orders Accounts.");
                return Err(Box::new(CliError::ClientError(e)));
            }
        };

    if orders_accounts.is_empty() {
        println!("No Orders Accounts found.");
        return Ok(CliResult {});
    }

    let markets = ctx.perp_markets.read().await;

    println!(
        "\n| {:^10} | {:^35} | {:^4} | {:^15} | {:^15} | {:^15} |",
        "Name", "Order ID", "Side", "Base Qty.", "Notional Size", "Price",
    );

    for (pubkey, account) in orders_accounts.iter() {
        let orders_account = get_zero_copy_account::<OrdersAccount>(&account.data);
        let market = match markets.iter().find(|m| m.address == orders_account.market) {
            Some(m) => m,
            None => {
                continue;
            }
        };
        let market_name = from_utf8(&market.state.inner.market_name)
            .unwrap()
            .trim_matches(char::from(0));

        let book = match AgnosticOrderBookContext::load(
            rpc_client,
            &market.state,
            &market.address,
            &market.state.inner.bids,
            &market.state.inner.asks,
        )
        .await
        {
            Ok(a) => a,
            Err(e) => {
                eprintln!("Failed to load Order Book Context.");
                return Err(Box::new(CliError::ContextError(e)));
            }
        };

        let book = book.state.read().await;
        let open_orders = orders_account.get_orders();

        for order in open_orders {
            let book_order = if order.side == Side::Ask {
                book.asks.iter().find(|p| p.order_id == order.order_id)
            } else {
                book.bids.iter().find(|p| p.order_id == order.order_id)
            };

            if book_order.is_some() {
                let bo = book_order.unwrap();
                println!(
                    "| {:^10} | {:^35} | {:<4} | {:>15} | {:>15} | {:>15} |",
                    market_name,
                    order.order_id,
                    order.side.to_string(),
                    fixed_to_ui(
                        I80F48::from(bo.base_quantity),
                        market.state.inner.config.decimals
                    ),
                    fixed_to_ui(I80F48::from(bo.quote_quantity), QUOTE_TOKEN_DECIMALS),
                    fixed_to_ui_price(
                        I80F48::from(bo.price),
                        market.state.inner.config.decimals,
                        QUOTE_TOKEN_DECIMALS
                    ),
                );
            } else {
                println!("{} | {:?}", order.order_id, order.side);
            };
        }
    }

    Ok(CliResult {})
}

pub async fn process_perps_cancel_order(
    config: &CliConfig,
    symbol: &str,
    order_id: u128,
    side: Side,
) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();
    let keypair = config.keypair.as_ref().unwrap();

    let (public_clearing, _) = derive_public_clearing_address();

    let ctx = match CypherContext::load_perpetual_markets(rpc_client).await {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Failed to load Cypher Context.");
            return Err(Box::new(CliError::ContextError(e)));
        }
    };

    let markets = ctx.perp_markets.read().await;
    let market = markets
        .iter()
        .find(|m| {
            from_utf8(&m.state.inner.market_name)
                .unwrap()
                .trim_matches('\0')
                == symbol
        })
        .unwrap();
    let market_name = from_utf8(&market.state.inner.market_name)
        .unwrap()
        .trim_matches(char::from(0));

    let (master_account, _) = derive_account_address(&keypair.pubkey(), 0); // TODO: change this, allow multiple accounts
    let (sub_account, _) = derive_sub_account_address(&master_account, 0); // TODO: change this, allow multiple accounts
    let (orders_account, _) = derive_orders_account_address(&market.address, &master_account);

    let encoded_pool_name = encode_string("USDC");
    let (quote_pool, _) = derive_pool_address(&encoded_pool_name);
    let (quote_pool_node, _) = derive_pool_node_address(&quote_pool, 0); // TODO: change this

    let args = CancelOrderArgs {
        order_id,
        side,
        is_client_id: false,
    };

    println!(
        "Cancelling order with ID {} on Perp Market {}",
        order_id, market_name
    );

    let ixs = vec![cancel_perp_order(
        &public_clearing,
        &cache_account::id(),
        &master_account,
        &sub_account,
        &market.address,
        &orders_account,
        &market.state.inner.orderbook,
        &market.state.inner.event_queue,
        &market.state.inner.bids,
        &market.state.inner.asks,
        &quote_pool_node,
        &keypair.pubkey(),
        args,
    )];

    let sig = match send_transactions(&rpc_client, ixs, keypair, true).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to submit transaction.");
            return Err(Box::new(CliError::ClientError(e)));
        }
    };

    println!(
        "Successfully cancelled order. Transaction signtaure: {}",
        sig.first().unwrap()
    );

    Ok(CliResult {})
}

pub async fn process_perps_market_order(
    config: &CliConfig,
    symbol: &str,
    side: Side,
    size: I80F48,
) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();
    let keypair = config.keypair.as_ref().unwrap();

    let (public_clearing, _) = derive_public_clearing_address();

    let ctx = match CypherContext::load_perpetual_markets(rpc_client).await {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Failed to load Cypher Context.");
            return Err(Box::new(CliError::ContextError(e)));
        }
    };

    let markets = ctx.perp_markets.read().await;
    let market = markets
        .iter()
        .find(|m| {
            from_utf8(&m.state.inner.market_name)
                .unwrap()
                .trim_matches('\0')
                == symbol
        })
        .unwrap();
    let market_name = from_utf8(&market.state.inner.market_name)
        .unwrap()
        .trim_matches(char::from(0));

    let (master_account, _) = derive_account_address(&keypair.pubkey(), 0); // TODO: change this, allow multiple accounts
    let (sub_account, _) = derive_sub_account_address(&master_account, 0); // TODO: change this, allow multiple accounts
    let (orders_account, _) = derive_orders_account_address(&market.address, &master_account);

    let encoded_pool_name = encode_string("USDC");
    let (quote_pool, _) = derive_pool_address(&encoded_pool_name);
    let (quote_pool_node, _) = derive_pool_node_address(&quote_pool, 0); // TODO: change this

    let orders_account_state = match get_or_create_orders_account(
        &rpc_client,
        keypair,
        &master_account,
        &market.address,
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

    let ob_ctx = match AgnosticOrderBookContext::load(
        rpc_client,
        &market.state,
        &market.address,
        &market.state.inner.bids,
        &market.state.inner.asks,
    )
    .await
    {
        Ok(ob) => ob,
        Err(e) => {
            return Err(Box::new(CliError::ContextError(e)));
        }
    };

    let impact_price = ob_ctx.get_impact_price(size.abs().to_num(), side).await;

    if impact_price.is_none() {
        return Err(Box::new(CliError::BadParameters(
            format!(
                "There is no available liquidity on the Order Book for Perp Market: {} to match size: {}",
                symbol,
                size
            )
        )));
    }

    let limit_price = impact_price.unwrap();

    let max_base_qty = size
        .mul(I80F48::from(
            10u64.pow(market.state.inner.config.decimals as u32),
        ))
        .to_num();

    let max_quote_qty = max_base_qty * limit_price;

    println!("{} - {} - {}", limit_price, max_base_qty, max_quote_qty);
    println!(
        "Placing market {} order on {} at price {:.5} with size {:.5} for total quote quantity of {:.5}.",
        side.to_string(),
        market_name,
        fixed_to_ui_price(
            I80F48::from(limit_price),
            market.state.inner.config.decimals,
            QUOTE_TOKEN_DECIMALS
        ),
        fixed_to_ui(
            I80F48::from(max_base_qty),
            market.state.inner.config.decimals
        ),
        fixed_to_ui(I80F48::from(max_quote_qty), QUOTE_TOKEN_DECIMALS)
    );

    let args = NewDerivativeOrderArgs {
        side,
        limit_price,
        max_base_qty,
        max_quote_qty,
        order_type: DerivativeOrderType::ImmediateOrCancel,
        self_trade_behavior: SelfTradeBehavior::CancelProvide,
        client_order_id: 0,
        limit: u16::MAX,
        max_ts: u64::MAX,
    };
    let ixs = vec![new_perp_order(
        &public_clearing,
        &cache_account::id(),
        &master_account,
        &sub_account,
        &market.address,
        &orders_account,
        &market.state.inner.orderbook,
        &market.state.inner.event_queue,
        &market.state.inner.bids,
        &market.state.inner.asks,
        &quote_pool_node,
        &keypair.pubkey(),
        args,
    )];

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

pub async fn process_perps_close(
    config: &CliConfig,
    symbol: &str,
) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();
    let keypair = config.keypair.as_ref().unwrap();

    let (public_clearing, _) = derive_public_clearing_address();

    let ctx = match CypherContext::load_perpetual_markets(rpc_client).await {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Failed to load Cypher Context.");
            return Err(Box::new(CliError::ContextError(e)));
        }
    };

    let markets = ctx.perp_markets.read().await;
    let market = markets
        .iter()
        .find(|m| {
            from_utf8(&m.state.inner.market_name)
                .unwrap()
                .trim_matches('\0')
                == symbol
        })
        .unwrap();
    let market_name = from_utf8(&market.state.inner.market_name)
        .unwrap()
        .trim_matches(char::from(0));

    let user_ctx = match UserContext::load(rpc_client, &keypair.pubkey(), None).await {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Failed to load User Context.");
            return Err(Box::new(CliError::ContextError(e)));
        }
    };

    let sub_account_state = match user_ctx.get_sub_account_with_position(&market.address) {
        Some(sa) => sa,
        None => {
            return Err(Box::new(CliError::BadParameters(format!(
                "Failed to get Sub Account with position for Perp Market: {}",
                symbol
            ))))
        }
    };

    let position = match sub_account_state.get_derivative_position(&market.address) {
        Some(p) => p,
        None => {
            return Err(Box::new(CliError::BadParameters(format!(
                "Failed to get Position for Perp Market: {}",
                symbol
            ))))
        }
    };

    // only fetch ob after making sure the position exists to minimze possible state changes
    let ob_ctx = match AgnosticOrderBookContext::load(
        rpc_client,
        &market.state,
        &market.address,
        &market.state.inner.bids,
        &market.state.inner.asks,
    )
    .await
    {
        Ok(ob) => ob,
        Err(e) => {
            return Err(Box::new(CliError::ContextError(e)));
        }
    };

    let position_size = position.total_position();
    let side = if position_size.is_negative() {
        Side::Bid
    } else {
        Side::Ask
    };
    let impact_price = ob_ctx
        .get_impact_price(
            fixed_to_ui(position_size.abs(), market.state.inner.config.decimals).to_num(),
            side,
        )
        .await;

    if impact_price.is_none() {
        return Err(Box::new(CliError::BadParameters(
            format!(
                "There is no available liquidity on the Order Book for Perp Market: {} to match size: {}",
                symbol,
                position_size
            )
        )));
    }

    let (master_account, _) = derive_account_address(&keypair.pubkey(), 0); // TODO: change this, allow multiple accounts
    let (sub_account, _) = derive_sub_account_address(&master_account, 0); // TODO: change this, allow multiple accounts
    let (orders_account, _) = derive_orders_account_address(&market.address, &master_account);

    let encoded_pool_name = encode_string("USDC");
    let (quote_pool, _) = derive_pool_address(&encoded_pool_name);
    let (quote_pool_node, _) = derive_pool_node_address(&quote_pool, 0); // TODO: change this

    let orders_account_state = match get_or_create_orders_account(
        &rpc_client,
        keypair,
        &master_account,
        &market.address,
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
    let limit_price = impact_price.unwrap();
    let max_base_qty = position_size.abs().to_num::<u64>();
    let mut max_quote_qty = max_base_qty * limit_price;
    max_quote_qty += (max_quote_qty * (10_000 + 30)) / 10_000; // TODO: change this to actually include the account's fee tier

    println!("{} - {} - {}", limit_price, max_base_qty, max_quote_qty);
    println!(
        "Closing Perp Position on {} at price {:.5} with size {:.5} for total quote quantity of {:.5}.",
        market_name,
        fixed_to_ui_price(
            I80F48::from(limit_price),
            market.state.inner.config.decimals,
            QUOTE_TOKEN_DECIMALS
        ),
        fixed_to_ui(
            I80F48::from(max_base_qty),
            market.state.inner.config.decimals
        ),
        fixed_to_ui(I80F48::from(max_quote_qty), QUOTE_TOKEN_DECIMALS)
    );

    let args = NewDerivativeOrderArgs {
        side,
        limit_price,
        max_base_qty,
        max_quote_qty,
        order_type: DerivativeOrderType::ImmediateOrCancel,
        self_trade_behavior: SelfTradeBehavior::CancelProvide,
        client_order_id: 0,
        limit: u16::MAX,
        max_ts: u64::MAX,
    };
    let ixs = vec![new_perp_order(
        &public_clearing,
        &cache_account::id(),
        &master_account,
        &sub_account,
        &market.address,
        &orders_account,
        &market.state.inner.orderbook,
        &market.state.inner.event_queue,
        &market.state.inner.bids,
        &market.state.inner.asks,
        &quote_pool_node,
        &keypair.pubkey(),
        args,
    )];

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

pub async fn process_perps_limit_order(
    config: &CliConfig,
    symbol: &str,
    side: Side,
    size: I80F48,
    price: I80F48,
) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();
    let keypair = config.keypair.as_ref().unwrap();

    let (public_clearing, _) = derive_public_clearing_address();

    let ctx = match CypherContext::load_perpetual_markets(rpc_client).await {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Failed to load Cypher Context.");
            return Err(Box::new(CliError::ContextError(e)));
        }
    };

    let markets = ctx.perp_markets.read().await;
    let market = markets
        .iter()
        .find(|m| {
            from_utf8(&m.state.inner.market_name)
                .unwrap()
                .trim_matches('\0')
                == symbol
        })
        .unwrap();

    let limit_price = convert_price_to_lots(
        price
            .mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32)))
            .to_num(),
        market.state.inner.base_multiplier,
        10u64.pow(market.state.inner.config.decimals as u32),
        market.state.inner.quote_multiplier,
    );

    let native_size = size
        .mul(I80F48::from(
            10u64.pow(market.state.inner.config.decimals as u32),
        ))
        .to_num();

    let (master_account, _) = derive_account_address(&keypair.pubkey(), 0); // TODO: change this, allow multiple accounts
    let (sub_account, _) = derive_sub_account_address(&master_account, 0); // TODO: change this, allow multiple accounts
    let (orders_account, _) = derive_orders_account_address(&market.address, &master_account);

    let encoded_pool_name = encode_string("USDC");
    let (quote_pool, _) = derive_pool_address(&encoded_pool_name);
    let (quote_pool_node, _) = derive_pool_node_address(&quote_pool, 0); // TODO: change this

    let orders_account_state = match get_or_create_orders_account(
        &rpc_client,
        keypair,
        &master_account,
        &market.address,
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

    let args = NewDerivativeOrderArgs {
        side,
        limit_price,
        max_base_qty: native_size,
        max_quote_qty: native_size * limit_price,
        order_type: DerivativeOrderType::PostOnly,
        self_trade_behavior: SelfTradeBehavior::CancelProvide,
        client_order_id: 0,
        limit: u16::MAX,
        max_ts: u64::MAX,
    };
    let ixs = vec![new_perp_order(
        &public_clearing,
        &cache_account::id(),
        &master_account,
        &sub_account,
        &market.address,
        &orders_account,
        &market.state.inner.orderbook,
        &market.state.inner.event_queue,
        &market.state.inner.bids,
        &market.state.inner.asks,
        &quote_pool_node,
        &keypair.pubkey(),
        args,
    )];

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

pub async fn process_perps_settle_funds(
    config: &CliConfig,
    symbol: &str,
) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();
    let keypair = config.keypair.as_ref().unwrap();

    let (public_clearing, _) = derive_public_clearing_address();

    let ctx = match CypherContext::load_perpetual_markets(rpc_client).await {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Failed to load Cypher Context.");
            return Err(Box::new(CliError::ContextError(e)));
        }
    };

    let markets = ctx.perp_markets.read().await;
    let market = markets
        .iter()
        .find(|m| {
            from_utf8(&m.state.inner.market_name)
                .unwrap()
                .trim_matches('\0')
                == symbol
        })
        .unwrap();

    let (master_account, _) = derive_account_address(&keypair.pubkey(), 0); // TODO: change this, allow multiple accounts
    let (sub_account, _) = derive_sub_account_address(&master_account, 0); // TODO: change this, allow multiple accounts
    let (orders_account, _) = derive_orders_account_address(&market.address, &master_account);

    let encoded_pool_name = encode_string("USDC");
    let (quote_pool, _) = derive_pool_address(&encoded_pool_name);
    let (quote_pool_node, _) = derive_pool_node_address(&quote_pool, 0); // TODO: change this

    let ixs = vec![settle_perp_funds(
        &public_clearing,
        &cache_account::id(),
        &master_account,
        &sub_account,
        &market.address,
        &orders_account,
        &quote_pool_node,
        &keypair.pubkey(),
    )];

    let sig = match send_transactions(&rpc_client, ixs, keypair, true).await {
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
