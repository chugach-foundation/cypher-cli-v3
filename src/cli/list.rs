use chrono::{DateTime, NaiveDateTime, Utc};
use clap::{App, ArgMatches, SubCommand};
use cypher_client::{
    utils::{fixed_to_ui, get_zero_copy_account},
    CacheAccount, FuturesMarket, PerpetualMarket, Pool,
};
use cypher_utils::utils::{get_cypher_zero_copy_account, get_program_accounts};

use solana_client::rpc_filter::RpcFilterType;

use std::{error, str::from_utf8};

use crate::cli::CliError;

use super::{command::CliCommand, CliConfig, CliResult};

#[derive(Debug)]
pub enum ListSubCommand {
    PerpetualMarkets,
    FuturesMarkets,
    Pools,
}

pub trait ListSubCommands {
    fn list_subcommands(self) -> Self;
}

impl ListSubCommands for App<'_, '_> {
    fn list_subcommands(self) -> Self {
        self.subcommand(
            SubCommand::with_name("list")
                .about("Lists a specific aspect of the platform.")
                .subcommand(
                    SubCommand::with_name("perps").about("Lists all existing Cypher Perp Markets."),
                )
                .subcommand(
                    SubCommand::with_name("futures")
                        .about("Lists all existing Cypher Futures Markets."),
                )
                .subcommand(
                    SubCommand::with_name("pools").about("Lists all existing Cypher Pools"),
                ),
        )
    }
}

pub fn parse_list_command(matches: &ArgMatches) -> Result<CliCommand, Box<dyn error::Error>> {
    match matches.subcommand() {
        ("perps", Some(_matches)) => Ok(CliCommand::List(ListSubCommand::PerpetualMarkets)),
        ("futures", Some(_matches)) => Ok(CliCommand::List(ListSubCommand::FuturesMarkets)),
        ("pools", Some(_matches)) => Ok(CliCommand::List(ListSubCommand::Pools)),
        ("", None) => {
            eprintln!("{}", matches.usage());
            Err(Box::new(CliError::CommandNotRecognized(
                "No market maker subcommand given.".to_string(),
            )))
        }
        _ => unreachable!(),
    }
}

#[allow(clippy::to_string_in_format_args)]
pub async fn list_futures_markets(config: &CliConfig) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();

    let cache_account = get_cypher_zero_copy_account::<CacheAccount>(
        rpc_client,
        &cypher_client::cache_account::id(),
    )
    .await
    .unwrap();

    let filters = vec![RpcFilterType::DataSize(
        std::mem::size_of::<FuturesMarket>() as u64 + 8,
    )];

    let program_accounts = get_program_accounts(rpc_client, filters, &cypher_client::ID)
        .await
        .unwrap();

    println!(
        "\n| {:^10} | {:^45} | {:^20} | {:^20} | {:^15} | {:^15} | {:^5} | {:^15} |",
        "Name",
        "Market",
        "Authority",
        "Expiry",
        "O. Price (UI)",
        "M. Price (UI)",
        "Dec.",
        "Pos. Count",
    );

    for (pubkey, account) in &program_accounts {
        let market = get_zero_copy_account::<FuturesMarket>(&account.data);
        let cache = cache_account.get_price_cache(market.inner.config.cache_index as usize);
        let market_name = from_utf8(&market.inner.market_name)
            .unwrap()
            .trim_matches(char::from(0));
        let expiry_dt = DateTime::<Utc>::from_utc(
            NaiveDateTime::from_timestamp(market.expires_at as i64, 0),
            Utc,
        );
        println!(
            "| {:<10} | {:^45} | {:^20} | {:^20} | {:>15.6} | {:>15.6} | {:>5} | {:>15} |",
            market_name,
            pubkey.to_string(),
            market.inner.authority.to_string()[..20].to_string(),
            expiry_dt.format("%d/%m/%Y %H:%M"),
            cache.oracle_price(),
            market.market_price(),
            market.inner.config.decimals,
            market.positions_count,
        );
    }

    println!("Listed {} Futures Markets.", program_accounts.len());

    Ok(CliResult {})
}

#[allow(clippy::to_string_in_format_args)]
pub async fn list_perp_markets(config: &CliConfig) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();

    let cache_account = get_cypher_zero_copy_account::<CacheAccount>(
        rpc_client,
        &cypher_client::cache_account::id(),
    )
    .await
    .unwrap();

    let filters = vec![RpcFilterType::DataSize(
        std::mem::size_of::<PerpetualMarket>() as u64 + 8,
    )];

    let program_accounts = get_program_accounts(rpc_client, filters, &cypher_client::ID)
        .await
        .unwrap();

    println!(
        "\n| {:^10} | {:^45} | {:^20} | {:^15} | {:^5} | {:^15} | {:^15} |",
        "Name", "Market", "Authority", "O. Price (UI)", "Dec.", "Long Funding", "Short Funding"
    );

    for (pubkey, account) in &program_accounts {
        let market = get_zero_copy_account::<PerpetualMarket>(&account.data);
        let cache = cache_account.get_price_cache(market.inner.config.cache_index as usize);
        let market_name = from_utf8(&market.inner.market_name)
            .unwrap()
            .trim_matches(char::from(0));
        println!(
            "| {:<10} | {:^45} | {:^20} | {:>15.5} | {:>5} | {:>15.10} | {:>15.10} |",
            market_name,
            pubkey.to_string(),
            market.inner.authority.to_string()[..20].to_string(),
            cache.oracle_price(),
            market.inner.config.decimals,
            market.long_funding(),
            market.short_funding(),
        );
    }

    println!("Listed {} Perpetual Markets.", program_accounts.len());

    Ok(CliResult {})
}

#[allow(clippy::to_string_in_format_args)]
pub async fn list_pools(config: &CliConfig) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();

    let cache_account = get_cypher_zero_copy_account::<CacheAccount>(
        rpc_client,
        &cypher_client::cache_account::id(),
    )
    .await
    .unwrap();

    let filters = vec![RpcFilterType::DataSize(
        std::mem::size_of::<Pool>() as u64 + 8,
    )];

    let program_accounts = get_program_accounts(rpc_client, filters, &cypher_client::ID)
        .await
        .unwrap();

    println!(
        "\n| {:^10} | {:^45} | {:^15} | {:^15} | {:^15} | {:^15} | {:^15} | {:^15} |",
        "Name",
        "Pool",
        "O. Price",
        "Deposits",
        "Borrows",
        "Deposit Rate",
        "Borrow Rate",
        "Util. Rate"
    );

    for (pubkey, account) in &program_accounts {
        let pool = get_zero_copy_account::<Pool>(&account.data);
        let cache = cache_account.get_price_cache(pool.config.cache_index as usize);
        let pool_name = from_utf8(&pool.pool_name)
            .unwrap()
            .trim_matches(char::from(0));
        println!(
            "| {:<10} | {:^45} | {:>15.4} | {:>15.2} | {:>15.2} | {:>15.4} | {:>15.4} | {:>15.4} |",
            pool_name,
            pubkey.to_string(),
            cache.oracle_price(),
            fixed_to_ui(pool.deposits(), pool.config.decimals),
            fixed_to_ui(pool.borrows(), pool.config.decimals),
            pool.deposit_rate() * 100,
            pool.borrow_rate() * 100,
            pool.utilization_rate() * 100
        );
    }

    println!("Listed {} Pools.", program_accounts.len());

    Ok(CliResult {})
}
