use clap::{App, Arg, ArgMatches, SubCommand};
use cypher_utils::logging::init_logger;
use log::info;
use std::{error, sync::Arc};
use tokio::sync::broadcast::channel;

use crate::{
    cli::CliError,
    market_maker::{config::Config, runner::Runner},
};

use super::{command::CliCommand, CliConfig, CliResult};

pub trait MarketMakerSubCommands {
    fn market_maker_subcommands(self) -> Self;
}

impl MarketMakerSubCommands for App<'_, '_> {
    fn market_maker_subcommands(self) -> Self {
        self.subcommand(
            SubCommand::with_name("market-maker")
                .about("Runs a market making bot.")
                .subcommand(
                    SubCommand::with_name("run").about("Runs the market maker.")
                        .arg(
                        Arg::with_name("config")
                            .short("c")
                            .long("config")
                            .value_name("FILE")
                            .takes_value(true)
                            .help("Filepath to a config. This config should follow the format displayed in `/cfg/market-maker/default.json`."),
                    )
                )
        )
    }
}

pub fn parse_market_maker_command(
    matches: &ArgMatches,
) -> Result<CliCommand, Box<dyn error::Error>> {
    match matches.subcommand() {
        ("run", Some(_matches)) => Ok(CliCommand::MarketMaker),
        ("", None) => {
            eprintln!("{}", matches.usage());
            Err(Box::new(CliError::CommandNotRecognized(
                "No market maker subcommand given.".to_string(),
            )))
        }
        _ => unreachable!(),
    }
}

pub async fn process_market_maker_command(
    config: &CliConfig,
) -> Result<CliResult, Box<dyn error::Error>> {
    _ = init_logger();
    let rpc_client = config.rpc_client.as_ref().unwrap();
    let pubsub_client = config.pubsub_client.as_ref().unwrap();

    info!("Hello Market Maker! 🙂");

    let shutdown_sender = Arc::new(channel::<bool>(1).0);

    let mm_runner = Runner::new(
        Arc::new(Config {}),
        Arc::clone(rpc_client),
        Arc::clone(pubsub_client),
        Arc::clone(&shutdown_sender),
    )
    .await;

    mm_runner.run().await;

    Ok(CliResult {})
}
