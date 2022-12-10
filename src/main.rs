use cli::CliResult;

mod cli;
mod common;
mod context;
mod liquidator;
mod market_maker;
mod random;
pub mod utils;

use {
    clap::{crate_description, crate_name, ArgMatches},
    cli::{app::get_clap_app, args::parse_args, command::process_command},
    std::error,
};

pub const VERSION: &str = "0.0.1";

#[tokio::main]
async fn main() {
    // let cli_hex = "30bd4824c71995bf0090290a00000000006400000000000000a8e0fa03000000000001b4eb906300000000ffffffffffffffffffff".to_string();
    // let fe_hex = "30bd4824c71995bf0090290a00000000006400000000000000a8e0fa03000000000001dbd717ee840100000000ffffffffffff1f00".to_string();

    // let cli_bytes = hex::decode(cli_hex).unwrap();
    // let fe_bytes = hex::decode(fe_hex).unwrap();

    // println!("CLI: {:?}", cli_bytes);
    // println!("FE: {:?}", fe_bytes);

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
