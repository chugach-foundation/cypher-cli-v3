use clap::{App, Arg, ArgMatches, SubCommand};
use cypher_client::Side;
use fixed::types::I80F48;
use std::{error, str::FromStr};

use crate::cli::CliError;

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
                        ),
                ),
        )
    }
}

pub fn parse_spot_command(matches: &ArgMatches) -> Result<CliCommand, Box<dyn error::Error>> {
    match matches.subcommand() {
        ("orders", Some(_matches)) => Ok(CliCommand::Spot(SpotSubCommand::Orders)),
        ("market", Some(_matches)) => {
            // market symbol
            let symbol = match matches.value_of("symbol") {
                Some(s) => s,
                None => {
                    return Err(Box::new(CliError::BadParameters(
                        "Symbol not provided, to see available markets try \"list futures\" command.".to_string(),
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
        ("place", Some(_matches)) => {
            // market symbol
            let symbol = match matches.value_of("symbol") {
                Some(s) => s,
                None => {
                    return Err(Box::new(CliError::BadParameters(
                        "Symbol not provided. To see available markets try \"list futures\" command.".to_string(),
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
            Ok(CliCommand::Spot(SpotSubCommand::Place {
                symbol: symbol.to_string(),
                side,
                size,
                price,
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
) -> Result<CliResult, Box<dyn error::Error>> {
    Ok(CliResult {})
}
