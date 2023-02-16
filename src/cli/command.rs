use clap::ArgMatches;
use solana_sdk::pubkey::Pubkey;
use std::error;

use crate::cli::CliError;

use super::{
    account::{
        close_account, create_account, create_whitelisted_account, parse_account_command,
        peek_account, AccountSubCommand,
    },
    faucet::{list_faucets, parse_faucet_command, request_faucet},
    futures::{
        list_futures_open_orders, parse_futures_command, process_futures_book,
        process_futures_cancel_order, process_futures_close, process_futures_limit_order,
        process_futures_market_order, process_futures_settle_funds, FuturesSubCommand,
    },
    liquidator::{parse_liquidator_command, process_liquidator_command},
    list::{
        list_futures_markets, list_perp_markets, list_pools, parse_list_command, ListSubCommand,
    },
    market_maker::{parse_market_maker_command, process_market_maker_command},
    perpetuals::{
        list_perps_open_orders, parse_perps_command, process_perps_book,
        process_perps_cancel_order, process_perps_close, process_perps_limit_order,
        process_perps_market_order, process_perps_settle_funds, PerpetualsSubCommand,
    },
    random::{parse_random_command, process_random_command},
    spot::{
        list_spot_open_orders, parse_spot_command, process_spot_book, process_spot_limit_order,
        process_spot_market_order, process_spot_settle_funds, SpotSubCommand,
    },
    sub_account::{
        close_sub_account, create_sub_account, deposit, parse_sub_account_command,
        peek_sub_account, transfer_between_sub_accounts, withdraw, SubAccountSubCommand,
    },
    CliConfig, CliResult,
};

#[derive(Debug)]
pub enum CliCommand {
    FaucetList,
    FaucetRequest {
        token_mint: Pubkey,
    },
    List(ListSubCommand),
    Account(AccountSubCommand),
    SubAccount(SubAccountSubCommand),
    Futures(FuturesSubCommand),
    Perpetuals(PerpetualsSubCommand),
    Spot(SpotSubCommand),
    Liquidator {
        config_path: String,
    },
    MarketMaker {
        config_path: String,
    },
    Random {
        interval: u64,
        output_keypair: Option<String>,
        input_keypair: Option<String>,
    },
}

pub fn parse_command(matches: &ArgMatches) -> Result<CliCommand, Box<dyn error::Error>> {
    let response = match matches.subcommand() {
        ("account", Some(matches)) => parse_account_command(matches),
        ("market-maker", Some(matches)) => parse_market_maker_command(matches),
        ("liquidator", Some(matches)) => parse_liquidator_command(matches),
        ("faucet", Some(matches)) => parse_faucet_command(matches),
        ("list", Some(matches)) => parse_list_command(matches),
        ("sub-account", Some(matches)) => parse_sub_account_command(matches),
        ("futures", Some(matches)) => parse_futures_command(matches),
        ("perps", Some(matches)) => parse_perps_command(matches),
        ("spot", Some(matches)) => parse_spot_command(matches),
        ("random", Some(matches)) => parse_random_command(matches),
        ("", None) => {
            eprintln!("{}", matches.usage());
            return Err(Box::new(CliError::CommandNotRecognized(
                "No subcommand given.".to_string(),
            )));
        }
        _ => unreachable!(),
    }?;
    Ok(response)
}

pub async fn process_command(config: &CliConfig) -> Result<CliResult, Box<dyn std::error::Error>> {
    match &config.command {
        CliCommand::Account(account_command) => match account_command {
            AccountSubCommand::Close { account_number } => {
                close_account(config, *account_number).await
            }
            AccountSubCommand::Create { account_number } => {
                create_account(config, *account_number).await
            }
            AccountSubCommand::CreateWhitelisted {
                private_clearing,
                whitelist,
                account_number,
            } => {
                create_whitelisted_account(config, private_clearing, whitelist, *account_number)
                    .await
            }
            AccountSubCommand::Peek {
                account_number,
                pubkey,
            } => peek_account(config, *account_number, *pubkey).await,
        },
        CliCommand::FaucetList => match list_faucets(config).await {
            Ok(r) => Ok(r),
            Err(e) => Err(e),
        },
        CliCommand::FaucetRequest { token_mint } => {
            match request_faucet(config, token_mint).await {
                Ok(r) => Ok(r),
                Err(e) => Err(Box::new(CliError::ClientError(e))),
            }
        }
        CliCommand::Liquidator { config_path } => {
            process_liquidator_command(config, config_path.as_str()).await
        }
        CliCommand::MarketMaker { config_path } => {
            process_market_maker_command(config, config_path.as_str()).await
        }
        CliCommand::Random {
            interval,
            output_keypair,
            input_keypair,
        } => process_random_command(config, *interval, output_keypair, input_keypair).await,
        CliCommand::List(list_command) => match list_command {
            ListSubCommand::PerpetualMarkets => list_perp_markets(config).await,
            ListSubCommand::FuturesMarkets => list_futures_markets(config).await,
            ListSubCommand::Pools => list_pools(config).await,
        },
        CliCommand::SubAccount(sub_account_command) => match sub_account_command {
            SubAccountSubCommand::Close {
                account_number,
                sub_account_number,
            } => close_sub_account(config, *account_number, *sub_account_number).await,
            SubAccountSubCommand::Create {
                account_number,
                sub_account_number,
                sub_account_alias,
            } => {
                create_sub_account(
                    config,
                    *account_number,
                    *sub_account_number,
                    sub_account_alias,
                )
                .await
            }
            SubAccountSubCommand::Peek {
                account_number,
                sub_account_number,
                pubkey,
            } => peek_sub_account(config, *account_number, *sub_account_number, *pubkey).await,
            SubAccountSubCommand::Deposit {
                account_number,
                sub_account_number,
                symbol,
                amount,
            } => {
                deposit(
                    config,
                    *account_number,
                    *sub_account_number,
                    symbol.as_str(),
                    *amount,
                )
                .await
            }
            SubAccountSubCommand::Withdraw {
                account_number,
                sub_account_number,
                symbol,
                amount,
            } => {
                withdraw(
                    config,
                    *account_number,
                    *sub_account_number,
                    symbol.as_str(),
                    *amount,
                )
                .await
            }
            SubAccountSubCommand::Transfer {
                account_number,
                from_sub_account_number,
                to_sub_account_number,
                symbol,
                amount,
            } => {
                transfer_between_sub_accounts(
                    config,
                    *account_number,
                    *from_sub_account_number,
                    *to_sub_account_number,
                    symbol.as_str(),
                    *amount,
                )
                .await
            }
        },
        CliCommand::Futures(futures_command) => match futures_command {
            FuturesSubCommand::Orders { pubkey } => list_futures_open_orders(config, *pubkey).await,
            FuturesSubCommand::Cancel {
                symbol,
                order_id,
                side,
            } => process_futures_cancel_order(config, symbol.as_str(), *order_id, *side).await,
            FuturesSubCommand::Close { symbol } => {
                process_futures_close(config, symbol.as_str()).await
            }
            FuturesSubCommand::Settle { symbol } => {
                process_futures_settle_funds(config, symbol.as_str()).await
            }
            FuturesSubCommand::Book { symbol } => {
                process_futures_book(config, symbol.as_str()).await
            }
            FuturesSubCommand::Market { symbol, side, size } => {
                process_futures_market_order(config, symbol.as_str(), *side, *size).await
            }
            FuturesSubCommand::Place {
                symbol,
                side,
                size,
                price,
                order_type,
            } => {
                process_futures_limit_order(
                    config,
                    symbol.as_str(),
                    *side,
                    *size,
                    *price,
                    order_type.as_str(),
                )
                .await
            }
        },
        CliCommand::Perpetuals(perps_command) => match perps_command {
            PerpetualsSubCommand::Orders { pubkey } => {
                list_perps_open_orders(config, *pubkey).await
            }
            PerpetualsSubCommand::Cancel {
                symbol,
                order_id,
                side,
            } => process_perps_cancel_order(config, symbol.as_str(), *order_id, *side).await,
            PerpetualsSubCommand::Close { symbol } => {
                process_perps_close(config, symbol.as_str()).await
            }
            PerpetualsSubCommand::Settle { symbol, pubkey } => {
                process_perps_settle_funds(config, symbol.as_str(), *pubkey).await
            }
            PerpetualsSubCommand::Book { symbol } => {
                process_perps_book(config, symbol.as_str()).await
            }
            PerpetualsSubCommand::Market { symbol, side, size } => {
                process_perps_market_order(config, symbol.as_str(), *side, *size).await
            }
            PerpetualsSubCommand::Place {
                symbol,
                side,
                size,
                price,
                order_type,
            } => {
                process_perps_limit_order(
                    config,
                    symbol.as_str(),
                    *side,
                    *size,
                    *price,
                    order_type.as_str(),
                )
                .await
            }
        },
        CliCommand::Spot(spot_command) => match spot_command {
            SpotSubCommand::Book { symbol } => process_spot_book(config, symbol.as_str()).await,
            SpotSubCommand::Orders { pubkey } => list_spot_open_orders(config, *pubkey).await,
            SpotSubCommand::Settle { symbol } => {
                process_spot_settle_funds(config, symbol.as_str()).await
            }
            SpotSubCommand::Market { symbol, side, size } => {
                process_spot_market_order(config, symbol.as_str(), *side, *size).await
            }
            SpotSubCommand::Place {
                symbol,
                side,
                size,
                price,
                order_type,
            } => {
                process_spot_limit_order(
                    config,
                    symbol.as_str(),
                    *side,
                    *size,
                    *price,
                    order_type.as_str(),
                )
                .await
            }
        },
    }
}
