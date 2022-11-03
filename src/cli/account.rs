use std::{error, str::FromStr};

use clap::{App, Arg, ArgMatches, SubCommand};
use cypher_client::{
    instructions::{
        close_account as close_account_ix, create_account as create_account_ix,
        create_whitelisted_account as create_whitelisted_account_ix,
    },
    utils::{derive_account_address, derive_public_clearing_address},
};
use cypher_utils::utils::create_transaction;
use solana_sdk::{pubkey::Pubkey, signer::Signer};

use super::{command::CliCommand, CliConfig, CliError, CliResult};

#[derive(Debug)]
pub enum AccountSubCommand {
    Close {
        account_number: Option<u8>,
    },
    Create {
        account_number: Option<u8>,
    },
    CreateWhitelisted {
        private_clearing: Pubkey,
        whitelist: Pubkey,
        account_number: Option<u8>,
    },
    Peek {
        account_number: Option<u8>,
    },
}

pub trait AccountSubCommands {
    fn account_subcommands(self) -> Self;
}

impl AccountSubCommands for App<'_, '_> {
    fn account_subcommands(self) -> Self {
        self.subcommand(
            SubCommand::with_name("account")
                .about("Subcommands that interact with Cypher Accounts.")
                .subcommand(
                    SubCommand::with_name("create")
                        .about("Creates a Cypher Account. If an account number is provided, that account will be created.")
                        .arg(
                            Arg::with_name("account-number")
                                .short("n")
                                .long("account-number")
                                .takes_value(true)
                                .help("The Account number, value should fit in a u8."),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("close")
                        .about("Closes a Cypher Account. If an account number is provided, that account will be closed.")
                        .arg(
                            Arg::with_name("account-number")
                                .short("n")
                                .long("account-number")
                                .takes_value(true)
                                .help("The Account number, value should fit in a u8."),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("create-whitelisted")
                        .about("Creates a Whitelisted Cypher Account. If an account number is provided, that account will be created.")
                        .arg(
                            Arg::with_name("private-clearing")
                                .short("c")
                                .long("private-clearing")
                                .takes_value(true)
                                .help("The Private Clearing Account, value should be a pubkey."),
                        )
                        .arg(
                            Arg::with_name("whitelist")
                                .short("w")
                                .long("whitelist")
                                .takes_value(true)
                                .help("The Whitelist Account, value should be a pubkey."),
                        )
                        .arg(
                            Arg::with_name("account-number")
                                .short("n")
                                .long("account-number")
                                .takes_value(true)
                                .help("The Account number, value should fit in a u8."),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("peek")
                        .about("Lists the Account's existing Sub Accounts and global margin ratios.")
                        .arg(
                            Arg::with_name("account-number")
                                .short("n")
                                .long("account-number")
                                .takes_value(true)
                                .help("The Account number, value should fit in a u8."),
                        ),
                ),
        )
    }
}

pub fn parse_account_command(matches: &ArgMatches) -> Result<CliCommand, Box<dyn error::Error>> {
    match matches.subcommand() {
        ("close", Some(matches)) => {
            let account_number = match matches.value_of("account-number") {
                Some(a) => Some(u8::from_str(a).unwrap()),
                None => None,
            };
            Ok(CliCommand::Account(AccountSubCommand::Close {
                account_number,
            }))
        }
        ("create", Some(matches)) => {
            let account_number = match matches.value_of("account-number") {
                Some(a) => Some(u8::from_str(a).unwrap()),
                None => None,
            };
            Ok(CliCommand::Account(AccountSubCommand::Create {
                account_number,
            }))
        }
        ("create-whitelisted", Some(matches)) => {
            let account_number = match matches.value_of("account-number") {
                Some(a) => Some(u8::from_str(a).unwrap()),
                None => None,
            };
            let whitelist = Pubkey::from_str(matches.value_of("whitelist").unwrap()).unwrap();
            let private_clearing =
                Pubkey::from_str(matches.value_of("private-clearing").unwrap()).unwrap();
            Ok(CliCommand::Account(AccountSubCommand::CreateWhitelisted {
                private_clearing,
                whitelist,
                account_number,
            }))
        }
        ("peek", Some(matches)) => {
            let account_number = match matches.value_of("account-number") {
                Some(a) => Some(u8::from_str(a).unwrap()),
                None => None,
            };
            Ok(CliCommand::Account(AccountSubCommand::Peek {
                account_number,
            }))
        }
        ("", None) => {
            eprintln!("{}", matches.usage());
            Err(Box::new(CliError::CommandNotRecognized(
                "No Account subcommand given.".to_string(),
            )))
        }
        _ => unreachable!(),
    }
}

pub async fn create_account(
    config: &CliConfig,
    account_number: Option<u8>,
) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();
    let keypair = config.keypair.as_ref().unwrap();

    let (public_clearing, _) = derive_public_clearing_address();
    let (account, account_bump, account_number) = if account_number.is_some() {
        let (account, bump) = derive_account_address(&keypair.pubkey(), account_number.unwrap());
        (account, bump, account_number.unwrap())
    } else {
        // derive the first account is the account number is not passed in
        let (account, bump) = derive_account_address(&keypair.pubkey(), 0);
        (account, bump, 0)
    };
    println!("Creating Account: {}", account);

    let ix = create_account_ix(
        &public_clearing,
        &keypair.pubkey(),
        &keypair.pubkey(),
        &account,
        account_bump,
        account_number,
    );

    let blockhash = rpc_client.get_latest_blockhash().await?;

    let tx = create_transaction(blockhash, &[ix], &keypair, Some(&[&keypair]));

    let sig = rpc_client
        .send_and_confirm_transaction_with_spinner(&tx)
        .await?;

    println!("Sucessfully submitted transaction. Signature: {}", sig);

    Ok(CliResult {})
}

pub async fn create_whitelisted_account(
    config: &CliConfig,
    private_clearing: &Pubkey,
    whitelist: &Pubkey,
    account_number: Option<u8>,
) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();
    let keypair = config.keypair.as_ref().unwrap();

    let (account, account_bump, account_number) = if account_number.is_some() {
        let (account, bump) = derive_account_address(&keypair.pubkey(), account_number.unwrap());
        (account, bump, account_number.unwrap())
    } else {
        // derive the first account is the account number is not passed in
        let (account, bump) = derive_account_address(&keypair.pubkey(), 0);
        (account, bump, 0)
    };
    println!("Creating Account: {}", account);

    let ix = create_whitelisted_account_ix(
        &private_clearing,
        &whitelist,
        &keypair.pubkey(),
        &keypair.pubkey(),
        &account,
        account_bump,
        account_number,
    );

    let blockhash = rpc_client.get_latest_blockhash().await?;

    let tx = create_transaction(blockhash, &[ix], &keypair, Some(&[&keypair]));

    let sig = rpc_client
        .send_and_confirm_transaction_with_spinner(&tx)
        .await?;

    println!("Sucessfully submitted transaction. Signature: {}", sig);

    Ok(CliResult {})
}

pub async fn close_account(
    config: &CliConfig,
    account_number: Option<u8>,
) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();
    let keypair = config.keypair.as_ref().unwrap();

    let (public_clearing, _) = derive_public_clearing_address();
    let (account, _) = if account_number.is_some() {
        derive_account_address(&keypair.pubkey(), account_number.unwrap())
    } else {
        // derive the first account is the account number is not passed in
        derive_account_address(&keypair.pubkey(), 0)
    };
    println!("Closing Account: {}", account);

    let ix = close_account_ix(&account, &keypair.pubkey(), &keypair.pubkey());

    let blockhash = rpc_client.get_latest_blockhash().await?;

    let tx = create_transaction(blockhash, &[ix], &keypair, Some(&[&keypair]));

    let sig = rpc_client
        .send_and_confirm_transaction_with_spinner(&tx)
        .await?;

    println!("Sucessfully submitted transaction. Signature: {}", sig);

    Ok(CliResult {})
}

pub async fn peek_account(
    config: &CliConfig,
    account_number: Option<u8>,
) -> Result<CliResult, Box<dyn error::Error>> {
    Ok(CliResult {})
}
