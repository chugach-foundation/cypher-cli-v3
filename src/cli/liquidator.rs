use clap::{App, Arg, ArgMatches, SubCommand};
use cypher_utils::logging::init_logger;
use log::info;
use std::error;

use crate::cli::CliError;

use super::{command::CliCommand, CliConfig, CliResult};

pub trait LiquidatorSubCommands {
    fn liquidator_subcommands(self) -> Self;
}

impl LiquidatorSubCommands for App<'_, '_> {
    fn liquidator_subcommands(self) -> Self {
        self.subcommand(
            SubCommand::with_name("liquidator")
            .about("Runs a liquidator bot.")
            .arg(
                Arg::with_name("config")
                    .short("c")
                    .long("config")
                    .value_name("FILE")
                    .global(true)
                    .takes_value(true)
                    .help("Filepath to a config. This config should follow the format displayed in `/cfg/liquidator/default.json`."),
            )
        )
    }
}

pub fn parse_liquidator_command(matches: &ArgMatches) -> Result<CliCommand, Box<dyn error::Error>> {
    let response = match matches.subcommand() {
        ("liquidator", Some(_matches)) => Ok(CliCommand::Liquidator),
        ("", None) => {
            eprintln!("{}", matches.usage());
            Err(CliError::CommandNotRecognized(
                "no subcommand given".to_string(),
            ))
        }
        _ => unreachable!(),
    }?;
    Ok(response)
}

pub fn process_liquidator_command(config: &CliConfig) -> Result<CliResult, Box<dyn error::Error>> {
    _ = init_logger();

    info!("Hello Liquidator! 🙂");

    Ok(CliResult {})
}
