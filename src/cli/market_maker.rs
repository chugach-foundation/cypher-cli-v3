use clap::{App, Arg, ArgMatches, SubCommand};
use cypher_utils::logging::init_logger;
use log::{info, warn};
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
                            .help("Path to a config file. This config should follow the format displayed in `/cfg/market-maker/default.json`."),
                    )
                )
        )
    }
}

pub fn parse_market_maker_command(
    matches: &ArgMatches,
) -> Result<CliCommand, Box<dyn error::Error>> {
    match matches.subcommand() {
        ("run", Some(matches)) => {
            let path = match matches.value_of("config") {
                Some(s) => s,
                None => {
                    return Err(Box::new(CliError::BadParameters(
                        "Path to config path not provided.".to_string(),
                    )));
                }
            };
            Ok(CliCommand::MarketMaker {
                path: path.to_string(),
            })
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

pub async fn process_market_maker_command(
    config: &CliConfig,
    config_path: &str,
) -> Result<CliResult, Box<dyn error::Error>> {
    _ = init_logger();
    let rpc_client = config.rpc_client.as_ref().unwrap();
    let pubsub_client = config.pubsub_client.as_ref().unwrap();
    let keypair = config.keypair.as_ref().unwrap();

    info!("Setting up components from config..");

    // let shutdown_sender = Arc::new(channel::<bool>(1).0);

    // let mm_config = match load_config(config_path) {
    //     Ok(c) => c,
    //     Err(e) => {
    //         warn!("There was an error loading config: {}", e.to_string());
    //         return Err(Box::new(CliError::BadParameters(
    //             "Failed to load market maker config.".to_string(),
    //         )));
    //     }
    // };

    // let mut mm_runner = Runner::new(
    //     Arc::new(mm_config),
    //     Arc::clone(rpc_client),
    //     Arc::clone(pubsub_client),
    //     Arc::clone(&shutdown_sender),
    // )
    // .await;

    // mm_runner.prepare().await;

    // info!("Let's dance! 🔥💃");

    // mm_runner.run().await;

    Ok(CliResult {})
}
