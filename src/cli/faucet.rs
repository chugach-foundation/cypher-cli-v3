use anchor_spl::token::{spl_token};
use clap::{App, Arg, ArgMatches, SubCommand};
use cypher_client::utils::{get_program_account};
use cypher_utils::contexts::CypherContext;
use cypher_utils::utils::{get_program_accounts, send_transactions};
use faucet_client::{
    derive_faucet_address, derive_mint_authority_address, request as request_ix, Faucet,
};
use solana_client::{client_error::ClientError, rpc_filter::RpcFilterType};
use solana_sdk::{pubkey::Pubkey, signer::Signer};
use spl_associated_token_account::{
    get_associated_token_address, instruction::create_associated_token_account,
};
use std::str::from_utf8;
use std::{error, str::FromStr};

use crate::cli::{CliError, CliResult};

use super::{command::CliCommand, CliConfig};

pub trait FaucetSubCommands {
    fn faucet_subcommands(self) -> Self;
}

impl FaucetSubCommands for App<'_, '_> {
    fn faucet_subcommands(self) -> Self {
        self.subcommand(
            SubCommand::with_name("faucet")
                .about("Subcommands that interact with devnet Faucets.")
                .subcommand(
                    SubCommand::with_name("request")
                        .about("Requests tokens from a Faucet for the given Mint.")
                        .arg(
                            Arg::with_name("token-mint")
                                .short("m")
                                .long("token-mint")
                                .takes_value(true)
                                .help("The token Mint, value should be a pubkey."),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("list")
                        .about("Lists all existing Faucets and their Mints."),
                ),
        )
    }
}

pub fn parse_faucet_command(matches: &ArgMatches) -> Result<CliCommand, Box<dyn error::Error>> {
    let response = match matches.subcommand() {
        ("list", Some(_matches)) => Ok(CliCommand::FaucetList),
        ("request", Some(matches)) => {
            let token_mint = Pubkey::from_str(matches.value_of("token-mint").unwrap()).unwrap();
            Ok(CliCommand::FaucetRequest { token_mint })
        }
        ("", None) => {
            eprintln!("{}", matches.usage());
            Err(CliError::CommandNotRecognized(
                "No `faucet` subcommand given!".to_string(),
            ))
        }
        _ => unreachable!(),
    }?;
    Ok(response)
}

pub async fn request_faucet(config: &CliConfig, mint: &Pubkey) -> Result<CliResult, ClientError> {
    let keypair = config.keypair.as_ref().unwrap();
    let rpc_client = config.rpc_client.as_ref().unwrap();

    let (faucet_address, _faucet_bump) = derive_faucet_address(mint);
    let (mint_authority_address, _mint_authority_bump) = derive_mint_authority_address(mint);

    let token_account_address = get_associated_token_address(&keypair.pubkey(), mint);

    let token_account = rpc_client.get_token_account(&token_account_address).await;

    // will just assume that in case of error the token account doesn't exist and needs to be created
    // should be changed later but not a priority now
    let mut ixs = if token_account.is_err() {
        vec![create_associated_token_account(
            &keypair.pubkey(),
            &keypair.pubkey(),
            mint,
            &spl_token::id(),
        )]
    } else {
        Vec::new()
    };

    ixs.push(request_ix(
        &faucet_address,
        mint,
        &mint_authority_address,
        &token_account_address,
    ));

    let sig =
        send_transactions(&rpc_client, ixs, keypair, true, Some((1_400_000, 1)), None).await?;

    println!(
        "Sucessfully submitted transaction. Signature: {}",
        sig.first().unwrap()
    );

    Ok(CliResult {})
}

pub async fn list_faucets(config: &CliConfig) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();

    let filters = vec![RpcFilterType::DataSize(
        std::mem::size_of::<Faucet>() as u64 + 8,
    )];

    let mut program_accounts = get_program_accounts(&rpc_client, filters, &faucet_client::ID)
        .await
        .unwrap();

    let ctx = match CypherContext::load_pools(rpc_client).await {
        Ok(c) => c,
        Err(e) => {
            return Err(Box::new(e));
        }
    };
    let pools = ctx.pools.read().await;

    println!(
        "\n| {:^10} | {:^45} | {:^45} |",
        "Symbol", "Faucet", "Token Mint",
    );

    for (pubkey, account) in program_accounts.iter_mut() {
        let faucet = get_program_account::<Faucet>(&mut &account.data[..]);
        let pool = match pools.iter().find(|p| p.state.token_mint == faucet.mint) {
            Some(p) => p,
            None => {
                continue;
            }
        };
        let pool_name = from_utf8(&pool.state.pool_name)
            .unwrap()
            .trim_matches(char::from(0));
        println!(
            "| {:<10} | {:^45} | {:^45} |",
            pool_name,
            pubkey.to_string(),
            faucet.mint.to_string(),
        );
    }

    println!("\nSucessfully listed {} Faucets.", program_accounts.len());

    Ok(CliResult {})
}
