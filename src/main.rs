mod cli;
mod common;
mod config;
mod context;
mod liquidator;
mod market_maker;
mod oracle;
mod random;
pub mod utils;

use cli::CliResult;

use {
    clap::{crate_description, crate_name, ArgMatches},
    cli::{app::get_clap_app, args::parse_args, command::process_command},
    std::error,
};

pub const VERSION: &str = "0.0.1";

#[tokio::main]
async fn main() {
    let matches = get_clap_app(crate_name!(), crate_description!(), VERSION).get_matches();

    match run(&matches).await {
        Ok(_) => (),
        Err(e) => {
            println!("An error ocurred: {:?}", e.as_ref());
        }
    };
}

async fn run(matches: &ArgMatches<'_>) -> Result<CliResult, Box<dyn error::Error>> {
    let config = parse_args(matches).await?;
    process_command(&config).await
}
