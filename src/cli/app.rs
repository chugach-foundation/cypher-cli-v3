use clap::{App, AppSettings, Arg};
use solana_clap_utils::input_validators::is_url_or_moniker;

use super::{
    account::AccountSubCommands, faucet::FaucetSubCommands, futures::FuturesSubCommands,
    liquidator::LiquidatorSubCommands, list::ListSubCommands, market_maker::MarketMakerSubCommands,
    perpetuals::PerpetualsSubCommands, spot::SpotSubCommands, sub_account::SubAccountSubCommands,
};

pub fn get_clap_app<'a, 'b>(name: &str, about: &'a str, version: &'b str) -> App<'a, 'b> {
    App::new(name)
        .about(about)
        .version(version)
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .setting(AppSettings::UnifiedHelpMessage)
        .arg(
            Arg::with_name("json_rpc_url")
                .short("u")
                .long("url")
                .value_name("URL_OR_MONIKER")
                .takes_value(true)
                .global(true)
                .validator(is_url_or_moniker)
                .help("URL for Solana's JSON RPC API."),
        )
        .arg(
            Arg::with_name("pubsub_rpc_url")
                .short("p")
                .long("pubsub_rpc_url")
                .value_name("URL")
                .takes_value(true)
                .global(true)
                .help("URL for Solana's PubSub JSON RPC WS."),
        )
        .arg(
            Arg::with_name("keypair")
                .short("k")
                .long("keypair")
                .value_name("KEYPAIR")
                .global(true)
                .takes_value(true)
                .help("Filepath to a keypair. If not provided, the app will attempt to load the default Solana keypair."),
        )
        .account_subcommands()
        .faucet_subcommands()
        .futures_subcommands()
        .liquidator_subcommands()
        .market_maker_subcommands()
        .list_subcommands()
        .perps_subcommands()
        .spot_subcommands()
        .sub_account_subcommands()
}
