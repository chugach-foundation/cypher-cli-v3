use clap::{App, Arg, ArgMatches, SubCommand};
use cypher_utils::{logging::init_logger, utils::load_keypair};
use log::{info, warn};
use solana_sdk::{signature::Keypair, signer::Signer};
use std::{error, fs::File, io::Write, str::FromStr, sync::Arc};
use tokio::sync::broadcast::channel;

use crate::cli::CliError;

use super::{command::CliCommand, CliConfig, CliResult};

const DEFAULT_INTERVAL: u64 = 30;

pub trait RandomSubCommands {
    fn random_subcommands(self) -> Self;
}

impl RandomSubCommands for App<'_, '_> {
    fn random_subcommands(self) -> Self {
        self.subcommand(
            SubCommand::with_name("random")
                .about("Runs a bot that performs random actions. (THIS SHOULD ONLY BE USED ON DEVNET)")
                .subcommand(
                    SubCommand::with_name("run")
                    .arg(
                        Arg::with_name("interval")
                            .short("i")
                            .long("interval")
                            .takes_value(true)
                            .help("The interval at which the bot performs an action, value should fit in a u64.")   
                    )
                    .arg(
                        Arg::with_name("input-keypair")
                            .long("input-keypair")
                            .takes_value(true)
                            .help("Filename for the input keypair, value should be a string")
                    )
                    .arg(
                        Arg::with_name("output-keypair")
                            .short("o")
                            .long("output-keypair")
                            .takes_value(true)
                            .help("Filename for the output keypair, value should be a string. In case no input keypair is provided, one will be created specifically for this use case.")
                    )
                )
        )
    }
}

pub fn parse_random_command(matches: &ArgMatches) -> Result<CliCommand, Box<dyn error::Error>> {
    match matches.subcommand() {
        ("run", Some(matches)) => {
            let output_keypair = matches.value_of("output-keypair").map(|s| s.to_string());
            let input_keypair = matches.value_of("input-keypair").map(|s| s.to_string());
            let interval = match matches.value_of("interval") {
                Some(i) => u64::from_str(i).unwrap(),
                None => DEFAULT_INTERVAL,
            };
            Ok(CliCommand::Random {
                interval,
                output_keypair,
                input_keypair,
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

pub async fn process_random_command(
    config: &CliConfig,
    _interval: u64,
    output_keypair: &Option<String>,
    input_keypair: &Option<String>,
) -> Result<CliResult, Box<dyn error::Error>> {
    _ = init_logger();
    let _rpc_client = config.rpc_client.as_ref().unwrap();
    let _pubsub_client = config.pubsub_client.as_ref().unwrap();

    let keypair = if input_keypair.is_some() {
        let input_filename = input_keypair.as_ref().unwrap();
        load_keypair(&input_filename)?
    } else {
        Keypair::new()
    };

    if output_keypair.is_some() && input_keypair.is_none() {
        let output_filename = output_keypair.as_ref().unwrap();
        info!(
            "Writing keypair {} to {}.",
            keypair.pubkey(),
            output_filename
        );

        write_keypair(&keypair, output_filename)?;
    }

    info!("Setting up components from config..");

    let _shutdown_sender = Arc::new(channel::<bool>(1).0);

    info!("Let's dance! 🔥💃");

    Ok(CliResult {})
}

fn write_keypair(keypair: &Keypair, filename: &str) -> Result<(), Box<dyn error::Error>> {
    let mut file = match File::create(filename) {
        Ok(f) => f,
        Err(e) => {
            warn!("Failed to create file: {:?}", e.to_string());
            return Err(Box::new(e));
        }
    };

    let keypair_bytes = keypair.to_bytes();
    let bytes_string = format!("{:?}", keypair_bytes);
    println!("{:?}", bytes_string);

    file.write_all(bytes_string.as_bytes())?;

    Ok(())
}
