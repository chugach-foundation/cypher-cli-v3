use anchor_spl::token::{spl_token, TokenAccount};
use clap::{App, Arg, ArgMatches, SubCommand};
use cypher_client::{
    constants::{QUOTE_TOKEN_DECIMALS, SUB_ACCOUNT_ALIAS_LEN},
    instructions::{
        close_sub_account as close_sub_account_ix, create_sub_account as create_sub_account_ix,
        deposit_funds, withdraw_funds,
    },
    quote_mint,
    utils::{
        derive_account_address, derive_pool_node_vault_signer_address,
        derive_public_clearing_address, derive_sub_account_address, derive_token_address,
        fixed_to_ui, fixed_to_ui_price, native_to_ui, native_to_ui_price,
    },
    wrapped_sol, CacheAccount, CypherSubAccount, MarginCollateralRatioType,
};
use cypher_utils::{
    contexts::CypherContext,
    utils::{
        create_transaction, encode_string, get_create_account_ix, get_cypher_zero_copy_account,
    },
};
use fixed::types::I80F48;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, signature::Keypair, signer::Signer};
use std::{
    error,
    str::{from_utf8, FromStr},
};
use thousands::Separable;

use super::{command::CliCommand, CliConfig, CliError, CliResult};

#[derive(Debug)]
pub enum SubAccountSubCommand {
    Close {
        account_number: Option<u8>,
        sub_account_number: Option<u8>,
    },
    Create {
        account_number: Option<u8>,
        sub_account_number: Option<u8>,
        sub_account_alias: Option<String>,
    },
    Deposit {
        account_number: Option<u8>,
        sub_account_number: Option<u8>,
        symbol: String,
        amount: I80F48,
    },
    Peek {
        account_number: Option<u8>,
        sub_account_number: Option<u8>,
        pubkey: Option<Pubkey>,
    },
    Withdraw {
        account_number: Option<u8>,
        sub_account_number: Option<u8>,
        symbol: String,
        amount: I80F48,
    },
}

pub trait SubAccountSubCommands {
    fn sub_account_subcommands(self) -> Self;
}

impl SubAccountSubCommands for App<'_, '_> {
    fn sub_account_subcommands(self) -> Self {
        self.subcommand(
            SubCommand::with_name("sub-account")
                .about("Subcommands that interact with Cypher Sub Accounts.")
                .subcommand(
                    SubCommand::with_name("create")
                        .about("Creates a Cypher Sub Account. If an Account number is provided, that Account will be used to derive this Sub Account's address. If a Sub Account number is provided, that Sub Account will be created.")
                        .arg(
                            Arg::with_name("account-number")
                                .short("n")
                                .long("account-number")
                                .takes_value(true)
                                .help("The Account number, value should fit in a u8."),
                        )
                        .arg(
                            Arg::with_name("sub-account-number")
                                .short("s")
                                .long("sub-account-number")
                                .takes_value(true)
                                .help("The Sub Account number, value should fit in a u8."),
                        )
                        .arg(
                            Arg::with_name("sub-account-alias")
                                .short("a")
                                .long("sub-account-alias")
                                .takes_value(true)
                                .help("An alias for the Sub Account, string should be less than 32 bytes when utf8 encoded."),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("close")
                        .about("Closes a Cypher Sub Account. If an Account number is provided, that Account will be used to derive this Sub Account's address. If a Sub Account number is provided, that Sub Account will be closed.")
                        .arg(
                            Arg::with_name("account-number")
                                .short("n")
                                .long("account-number")
                                .takes_value(true)
                                .help("The Account number, value should fit in a u8."),
                        )
                        .arg(
                            Arg::with_name("sub-account-number")
                                .short("s")
                                .long("sub-account-number")
                                .takes_value(true)
                                .help("The Sub Account number, value should fit in a u8."),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("deposit")
                        .about("Deposits a given token into a Cypher Sub Account. If an Account number is provided, that Account will be used to derive this Sub Account's address. If a Sub Account number is provided, that Sub Account will be closed.")
                        .arg(
                            Arg::with_name("account-number")
                                .short("n")
                                .long("account-number")
                                .takes_value(true)
                                .help("The Account number, value should fit in a u8."),
                        )
                        .arg(
                            Arg::with_name("sub-account-number")
                                .short("s")
                                .long("sub-account-number")
                                .takes_value(true)
                                .help("The Sub Account number, value should fit in a u8."),
                        )
                        .arg(
                            Arg::with_name("symbol")
                                .long("symbol")
                                .takes_value(true)
                                .help("The symbol of the token, e.g \"SOL\"."),
                        )
                        .arg(
                            Arg::with_name("amount")
                                .short("a")
                                .long("amount")
                                .takes_value(true)
                                .help("The amount, value should be a number."),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("withdraw")
                        .about("Withdraws a given token from a Cypher Sub Account. If an Account number is provided, that Account will be used to derive this Sub Account's address. If a Sub Account number is provided, that Sub Account will be closed.")
                        .arg(
                            Arg::with_name("account-number")
                                .short("n")
                                .long("account-number")
                                .takes_value(true)
                                .help("The Account number, value should fit in a u8."),
                        )
                        .arg(
                            Arg::with_name("sub-account-number")
                                .short("s")
                                .long("sub-account-number")
                                .takes_value(true)
                                .help("The Sub Account number, value should fit in a u8."),
                        )
                        .arg(
                            Arg::with_name("symbol")
                                .long("symbol")
                                .takes_value(true)
                                .help("The symbol of the token, e.g \"SOL\"."),
                        )
                        .arg(
                            Arg::with_name("amount")
                                .short("a")
                                .long("amount")
                                .takes_value(true)
                                .help("The amount, value should be a number."),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("peek")
                        .about("Lists the Sub Account's existing positions and margin ratios.")
                        .arg(
                            Arg::with_name("account-number")
                                .short("n")
                                .long("account-number")
                                .takes_value(true)
                                .help("The Account number, value should fit in a u8."),
                        )
                        .arg(
                            Arg::with_name("sub-account-number")
                                .short("s")
                                .long("sub-account-number")
                                .takes_value(true)
                                .help("The Sub Account number, value should fit in a u8."),
                        )
                        .arg(
                            Arg::with_name("pubkey")
                                .long("pubkey")
                                .takes_value(true)
                                .help("The Sub Account pubkey, value should be a pubkey."),
                        ),
                ),
        )
    }
}

pub fn parse_sub_account_command(
    matches: &ArgMatches,
) -> Result<CliCommand, Box<dyn error::Error>> {
    match matches.subcommand() {
        ("close", Some(matches)) => {
            let account_number = match matches.value_of("account-number") {
                Some(a) => Some(u8::from_str(a).unwrap()),
                None => None,
            };
            let sub_account_number = match matches.value_of("sub-account-number") {
                Some(a) => Some(u8::from_str(a).unwrap()),
                None => None,
            };
            Ok(CliCommand::SubAccount(SubAccountSubCommand::Close {
                account_number,
                sub_account_number,
            }))
        }
        ("create", Some(matches)) => {
            let account_number = match matches.value_of("account-number") {
                Some(a) => Some(u8::from_str(a).unwrap()),
                None => None,
            };
            let sub_account_number = match matches.value_of("sub-account-number") {
                Some(a) => Some(u8::from_str(a).unwrap()),
                None => None,
            };
            let sub_account_alias = match matches.value_of("sub-account-alias") {
                Some(a) => Some(a.to_string()),
                None => None,
            };
            Ok(CliCommand::SubAccount(SubAccountSubCommand::Create {
                account_number,
                sub_account_number,
                sub_account_alias,
            }))
        }
        ("deposit", Some(matches)) => {
            let account_number = match matches.value_of("account-number") {
                Some(a) => Some(u8::from_str(a).unwrap()),
                None => None,
            };
            let sub_account_number = match matches.value_of("sub-account-number") {
                Some(a) => Some(u8::from_str(a).unwrap()),
                None => None,
            };
            let symbol = matches.value_of("symbol").unwrap().to_string();
            let amount = match matches.value_of("amount") {
                Some(s) => match I80F48::from_str(s) {
                    Ok(s) => s,
                    Err(e) => {
                        return Err(Box::new(CliError::BadParameters(format!(
                            "Invalid amount.: {}",
                            e.to_string()
                        ))));
                    }
                },
                None => {
                    return Err(Box::new(CliError::BadParameters(
                        "Amount not provided. value should be in token, non-native units."
                            .to_string(),
                    )));
                }
            };
            Ok(CliCommand::SubAccount(SubAccountSubCommand::Deposit {
                account_number,
                sub_account_number,
                symbol,
                amount,
            }))
        }
        ("withdraw", Some(matches)) => {
            let account_number = match matches.value_of("account-number") {
                Some(a) => Some(u8::from_str(a).unwrap()),
                None => None,
            };
            let sub_account_number = match matches.value_of("sub-account-number") {
                Some(a) => Some(u8::from_str(a).unwrap()),
                None => None,
            };
            let symbol = matches.value_of("symbol").unwrap().to_string();
            let amount = match matches.value_of("amount") {
                Some(s) => match I80F48::from_str(s) {
                    Ok(s) => s,
                    Err(e) => {
                        return Err(Box::new(CliError::BadParameters(format!(
                            "Invalid amount.: {}",
                            e.to_string()
                        ))));
                    }
                },
                None => {
                    return Err(Box::new(CliError::BadParameters(
                        "Amount not provided. value should be in token, non-native units."
                            .to_string(),
                    )));
                }
            };
            Ok(CliCommand::SubAccount(SubAccountSubCommand::Withdraw {
                account_number,
                sub_account_number,
                symbol,
                amount,
            }))
        }
        ("peek", Some(matches)) => {
            let account_number = match matches.value_of("account-number") {
                Some(a) => Some(u8::from_str(a).unwrap()),
                None => None,
            };
            let sub_account_number = match matches.value_of("sub-account-number") {
                Some(a) => Some(u8::from_str(a).unwrap()),
                None => None,
            };
            let pubkey = match matches.value_of("pubkey") {
                Some(a) => Some(Pubkey::from_str(a).unwrap()),
                None => None,
            };
            Ok(CliCommand::SubAccount(SubAccountSubCommand::Peek {
                account_number,
                sub_account_number,
                pubkey,
            }))
        }
        ("", None) => {
            eprintln!("{}", matches.usage());
            Err(Box::new(CliError::CommandNotRecognized(
                "No Sub Account subcommand given.".to_string(),
            )))
        }
        _ => unreachable!(),
    }
}

pub async fn create_sub_account(
    config: &CliConfig,
    account_number: Option<u8>,
    sub_account_number: Option<u8>,
    sub_account_alias: &Option<String>,
) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();
    let keypair = config.keypair.as_ref().unwrap();

    // derive clearing address
    let (public_clearing, _) = derive_public_clearing_address();

    // derive account address
    let (account, account_bump, account_number) = if account_number.is_some() {
        let (account, bump) = derive_account_address(&keypair.pubkey(), account_number.unwrap());
        (account, bump, account_number.unwrap())
    } else {
        // derive the first account is the account number is not passed in
        let (account, bump) = derive_account_address(&keypair.pubkey(), 0);
        (account, bump, 0)
    };
    println!("Using Account: {}", account);

    // derive sub account address
    let (sub_account, sub_account_bump, sub_account_number) = if sub_account_number.is_some() {
        let (account, bump) = derive_sub_account_address(&account, sub_account_number.unwrap());
        (account, bump, sub_account_number.unwrap())
    } else {
        // derive the first account is the account number is not passed in
        let (account, bump) = derive_sub_account_address(&account, 0);
        (account, bump, 0)
    };
    println!("Creating Sub Account: {}", sub_account);

    let sub_account_alias = if sub_account_alias.is_some() {
        println!("Sub Account Alias: {}", sub_account_alias.as_ref().unwrap());
        encode_string(sub_account_alias.as_ref().unwrap().as_str())
    } else {
        [0; SUB_ACCOUNT_ALIAS_LEN]
    };

    let ix = create_sub_account_ix(
        &keypair.pubkey(),
        &keypair.pubkey(),
        &account,
        &sub_account,
        sub_account_bump,
        sub_account_number,
        sub_account_alias,
    );

    let blockhash = rpc_client.get_latest_blockhash().await?;

    let tx = create_transaction(blockhash, &[ix], &keypair, Some(&[&keypair]));

    let sig = rpc_client
        .send_and_confirm_transaction_with_spinner(&tx)
        .await?;

    println!("Sucessfully submitted transaction. Signature: {}", sig);

    Ok(CliResult {})
}

pub async fn close_sub_account(
    config: &CliConfig,
    account_number: Option<u8>,
    sub_account_number: Option<u8>,
) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();
    let keypair = config.keypair.as_ref().unwrap();

    // derive account address
    let (account, _account_bump) = if account_number.is_some() {
        derive_account_address(&keypair.pubkey(), account_number.unwrap())
    } else {
        // derive the first account is the account number is not passed in
        derive_account_address(&keypair.pubkey(), 0)
    };
    println!("Using Account: {}", account);

    // derive sub account address
    let (sub_account, _sub_account_bump) = if sub_account_number.is_some() {
        derive_sub_account_address(&account, sub_account_number.unwrap())
    } else {
        // derive the first account is the account number is not passed in
        derive_sub_account_address(&account, 0)
    };
    println!("Closing Sub Account: {}", sub_account);

    let ix = close_sub_account_ix(&account, &sub_account, &keypair.pubkey(), &keypair.pubkey());

    let blockhash = rpc_client.get_latest_blockhash().await?;

    let tx = create_transaction(blockhash, &[ix], &keypair, Some(&[&keypair]));

    let sig = rpc_client
        .send_and_confirm_transaction_with_spinner(&tx)
        .await?;

    println!("Sucessfully submitted transaction. Signature: {}", sig);

    Ok(CliResult {})
}

pub async fn deposit(
    config: &CliConfig,
    account_number: Option<u8>,
    sub_account_number: Option<u8>,
    symbol: &str,
    amount: I80F48,
) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();
    let keypair = config.keypair.as_ref().unwrap();

    // derive clearing address
    let (public_clearing, _) = derive_public_clearing_address();

    // derive account address
    let (account, _account_bump) = if account_number.is_some() {
        derive_account_address(&keypair.pubkey(), account_number.unwrap())
    } else {
        // derive the first account is the account number is not passed in
        derive_account_address(&keypair.pubkey(), 0)
    };
    println!("Using Account: {}", account);

    // derive sub account address
    let (sub_account, _sub_account_bump) = if sub_account_number.is_some() {
        derive_sub_account_address(&account, sub_account_number.unwrap())
    } else {
        // derive the first account is the account number is not passed in
        derive_sub_account_address(&account, 0)
    };

    let mut ixs: Vec<Instruction> = Vec::new();

    let ctx = CypherContext::load(&rpc_client).await.unwrap();
    let pools = ctx.pools.read().await;
    let encoded_symbol = encode_string(symbol);

    let pool = pools
        .iter()
        .find(|p| p.state.pool_name == encoded_symbol)
        .unwrap();

    let pool_node_info = pool
        .state
        .nodes
        .iter()
        .find(|n| n.pool_node != Pubkey::default())
        .unwrap();

    // We will simply assume that the user has an ATA for the given token mint if it is not the Wrapped SOL mint
    let (source_token_account, token_account_keypair) = if pool.state.token_mint == wrapped_sol::ID
    {
        // In the case where this is a Wrapped SOL deposit we will need to create a token account with rent
        // plus however much we want to deposit before depositing
        let token_account = Keypair::new();
        ixs.extend(vec![
            get_create_account_ix(
                keypair,
                &token_account,
                TokenAccount::LEN,
                &spl_token::id(),
                Some(
                    amount
                        .checked_mul(I80F48::from(
                            10_u64
                                .checked_pow(pool.state.config.decimals as u32)
                                .unwrap(),
                        ))
                        .unwrap()
                        .to_num(),
                ),
            ),
            spl_token::instruction::initialize_account(
                &spl_token::id(),
                &token_account.pubkey(),
                &pool.state.token_mint,
                &keypair.pubkey(),
            )
            .unwrap(), // this is prone to blowing up, should do it some other way
        ]);
        (token_account.pubkey(), Some(token_account))
    } else {
        (
            derive_token_address(&keypair.pubkey(), &pool.state.token_mint),
            None,
        )
    };

    ixs.push(deposit_funds(
        &public_clearing,
        &cypher_client::cache_account::id(),
        &account,
        &sub_account,
        &pool.address,
        &pool_node_info.pool_node,
        &source_token_account,
        &pool.state.token_vault,
        &pool.state.token_mint,
        &keypair.pubkey(),
        amount
            .checked_mul(I80F48::from(
                10_u64
                    .checked_pow(pool.state.config.decimals as u32)
                    .unwrap(),
            ))
            .unwrap()
            .to_num(),
    ));

    // If it a Wrapped SOL deposit we can close the account after depositing
    if pool.state.token_mint == wrapped_sol::ID {
        ixs.push(
            spl_token::instruction::close_account(
                &spl_token::id(),
                &source_token_account,
                &keypair.pubkey(),
                &keypair.pubkey(),
                &[&keypair.pubkey()],
            )
            .unwrap(), // this too is prone to blowing up, should be done some other way
        );
    }

    let blockhash = match rpc_client.get_latest_blockhash().await {
        Ok(h) => h,
        Err(e) => {
            return Err(Box::new(CliError::ClientError(e)));
        }
    };

    let tx = if token_account_keypair.is_some() {
        create_transaction(
            blockhash,
            &ixs,
            keypair,
            Some(&[&token_account_keypair.unwrap()]),
        )
    } else {
        create_transaction(blockhash, &ixs, keypair, None)
    };

    let sig = rpc_client
        .send_and_confirm_transaction_with_spinner(&tx)
        .await?;

    println!("Sucessfully submitted transaction. Signature: {}", sig);

    Ok(CliResult {})
}

pub async fn withdraw(
    config: &CliConfig,
    account_number: Option<u8>,
    sub_account_number: Option<u8>,
    symbol: &str,
    amount: I80F48,
) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();
    let keypair = config.keypair.as_ref().unwrap();

    // derive clearing address
    let (public_clearing, _) = derive_public_clearing_address();

    // derive account address
    let (account, _account_bump) = if account_number.is_some() {
        derive_account_address(&keypair.pubkey(), account_number.unwrap())
    } else {
        // derive the first account is the account number is not passed in
        derive_account_address(&keypair.pubkey(), 0)
    };
    println!("Using Account: {}", account);

    // derive sub account address
    let (sub_account, _sub_account_bump) = if sub_account_number.is_some() {
        derive_sub_account_address(&account, sub_account_number.unwrap())
    } else {
        // derive the first account is the account number is not passed in
        derive_sub_account_address(&account, 0)
    };

    let mut ixs: Vec<Instruction> = Vec::new();

    let ctx = CypherContext::load(&rpc_client).await.unwrap();
    let pools = ctx.pools.read().await;
    let encoded_symbol = encode_string(symbol);

    let pool = pools
        .iter()
        .find(|p| p.state.pool_name == encoded_symbol)
        .unwrap();

    let pool_node_info = pool
        .state
        .nodes
        .iter()
        .find(|n| n.pool_node != Pubkey::default())
        .unwrap();

    let (vault_signer, _) = derive_pool_node_vault_signer_address(&pool_node_info.pool_node);

    // We will simply assume that the user has an ATA for the given token mint if it is not the Wrapped SOL mint
    let (destination_token_account, token_account_keypair) =
        if pool.state.token_mint == wrapped_sol::ID {
            // In the case where this is a Wrapped SOL deposit we will need to create a token account with rent
            // plus however much we want to deposit before depositing
            let token_account = Keypair::new();
            ixs.extend(vec![
                get_create_account_ix(
                    keypair,
                    &token_account,
                    TokenAccount::LEN,
                    &spl_token::id(),
                    None,
                ),
                spl_token::instruction::initialize_account(
                    &spl_token::id(),
                    &token_account.pubkey(),
                    &pool.state.token_mint,
                    &keypair.pubkey(),
                )
                .unwrap(), // this is prone to blowing up, should do it some other way
            ]);
            (token_account.pubkey(), Some(token_account))
        } else {
            (
                derive_token_address(&keypair.pubkey(), &pool.state.token_mint),
                None,
            )
        };

    ixs.push(withdraw_funds(
        &public_clearing,
        &cypher_client::cache_account::id(),
        &account,
        &sub_account,
        &pool.address,
        &pool_node_info.pool_node,
        &destination_token_account,
        &pool.state.token_vault,
        &vault_signer,
        &pool.state.token_mint,
        &keypair.pubkey(),
        amount
            .checked_mul(I80F48::from(
                10_u64
                    .checked_pow(pool.state.config.decimals as u32)
                    .unwrap(),
            ))
            .unwrap()
            .to_num(),
    ));

    // If it a Wrapped SOL deposit we can close the account after withdrawing so the SOL goes to the wallet
    if pool.state.token_mint == wrapped_sol::ID {
        ixs.push(
            spl_token::instruction::close_account(
                &spl_token::id(),
                &destination_token_account,
                &keypair.pubkey(),
                &keypair.pubkey(),
                &[&keypair.pubkey()],
            )
            .unwrap(), // this too is prone to blowing up, should be done some other way
        );
    }

    let blockhash = match rpc_client.get_latest_blockhash().await {
        Ok(h) => h,
        Err(e) => {
            return Err(Box::new(CliError::ClientError(e)));
        }
    };

    let tx = if token_account_keypair.is_some() {
        create_transaction(
            blockhash,
            &ixs,
            keypair,
            Some(&[&token_account_keypair.unwrap()]),
        )
    } else {
        create_transaction(blockhash, &ixs, keypair, None)
    };

    let sig = rpc_client
        .send_and_confirm_transaction_with_spinner(&tx)
        .await?;

    println!("Sucessfully submitted transaction. Signature: {}", sig);

    Ok(CliResult {})
}

pub async fn peek_sub_account(
    config: &CliConfig,
    account_number: Option<u8>,
    sub_account_number: Option<u8>,
    pubkey: Option<Pubkey>,
) -> Result<CliResult, Box<dyn error::Error>> {
    let rpc_client = config.rpc_client.as_ref().unwrap();
    let keypair = config.keypair.as_ref().unwrap();

    let sub_account = if pubkey.is_some() {
        pubkey.unwrap()
    } else {
        // derive account address
        let (account, _account_bump) = if account_number.is_some() {
            derive_account_address(&keypair.pubkey(), account_number.unwrap())
        } else {
            // derive the first account is the account number is not passed in
            derive_account_address(&keypair.pubkey(), 0)
        };
        println!("Using Account: {}", account);

        // derive sub account address
        let (sub_account, _sub_account_bump) = if sub_account_number.is_some() {
            derive_sub_account_address(&account, sub_account_number.unwrap())
        } else {
            // derive the first account is the account number is not passed in
            derive_sub_account_address(&account, 0)
        };
        sub_account
    };
    println!("Using Sub Account: {}", sub_account);

    let cache_account = get_cypher_zero_copy_account::<CacheAccount>(
        &rpc_client,
        &cypher_client::cache_account::id(),
    )
    .await
    .unwrap();

    let sub_account = get_cypher_zero_copy_account::<CypherSubAccount>(&rpc_client, &sub_account)
        .await
        .unwrap();

    let ctx = CypherContext::load(&rpc_client).await.unwrap();

    let pools = ctx.pools.read().await;
    let futures_markets = ctx.futures_markets.read().await;
    let perp_markets = ctx.perp_markets.read().await;

    println!(
        "\n| {:^5} | {:^10} | {:^25} | {:^25} | {:^25} | {:^15} | {:^15} | {:^15} | {:^15} |",
        "Idx",
        "Position",
        "Size (UI)",
        "O. Price (UI)",
        "Value (UI)",
        "OO B. Total",
        "OO B. Locked",
        "OO Q. Total",
        "OO Q. Locked"
    );
    for (pos_idx, pos) in sub_account.positions.iter().enumerate() {
        if pos.spot.token_mint != Pubkey::default() {
            let pool = pools
                .iter()
                .find(|p| p.state.token_mint == pos.spot.token_mint)
                .unwrap();
            let pool_name = from_utf8(&pool.state.pool_name)
                .unwrap()
                .trim_matches(char::from(0));
            let cache = cache_account.get_price_cache(pool.state.config.cache_index as usize);
            let price = cache.oracle_price();
            let total_position =
                fixed_to_ui(pos.spot.total_position(cache), pool.state.config.decimals);
            let position_value = total_position.checked_mul(price).unwrap();
            println!(
                "| {:^5} | {:^10} | {:>25.4} | {:>25.4} | {:>25.4} | {:>15} | {:>15} | {:>15} | {:>15} |",
                pos_idx,
                pool_name,
                total_position,
                price,
                position_value,
                native_to_ui(pos.spot.open_orders_cache.coin_total, pool.state.config.decimals),
                native_to_ui(pos.spot.open_orders_cache.coin_total - pos.spot.open_orders_cache.coin_free, pool.state.config.decimals),
                native_to_ui(pos.spot.open_orders_cache.pc_total, QUOTE_TOKEN_DECIMALS),
                native_to_ui(pos.spot.open_orders_cache.pc_total - pos.spot.open_orders_cache.pc_free, QUOTE_TOKEN_DECIMALS),
            );
        }
    }
    for (pos_idx, pos) in sub_account.positions.iter().enumerate() {
        if pos.derivative.market != Pubkey::default() {
            let futures_market = futures_markets
                .iter()
                .find(|m| m.address == pos.derivative.market);
            if futures_market.is_some() {
                let market = futures_market.unwrap();
                let market_name = from_utf8(&market.state.inner.market_name)
                    .unwrap()
                    .trim_matches(char::from(0));
                let market_config = market.state.inner.config;
                let cache =
                    cache_account.get_price_cache(market.state.inner.config.cache_index as usize);
                let price = cache.market_price();
                let total_position =
                    fixed_to_ui(pos.derivative.base_position(), market_config.decimals);
                let position_value = total_position.checked_mul(price).unwrap();
                println!(
                    "| {:^5} | {:^10} | {:>25.4} | {:>25.4} | {:>25.4} | {:>15} | {:>15} | {:>15} | {:>15} |",
                    pos_idx,
                    market_name,
                    total_position,
                    price,
                    position_value,
                    native_to_ui(pos.derivative.open_orders_cache.coin_total, market.state.inner.config.decimals),
                    native_to_ui(pos.derivative.open_orders_cache.coin_total
                        - pos.derivative.open_orders_cache.coin_free, market.state.inner.config.decimals),
                    native_to_ui(pos.derivative.open_orders_cache.pc_total, QUOTE_TOKEN_DECIMALS),
                    native_to_ui(pos.derivative.open_orders_cache.pc_total - pos.derivative.open_orders_cache.pc_free, QUOTE_TOKEN_DECIMALS),
                );
            }

            let perp_market = perp_markets
                .iter()
                .find(|m| m.address == pos.derivative.market);
            if perp_market.is_some() {
                let market = perp_market.unwrap();
                let market_name = from_utf8(&market.state.inner.market_name)
                    .unwrap()
                    .trim_matches(char::from(0));
                let market_config = market.state.inner.config;
                let cache =
                    cache_account.get_price_cache(market.state.inner.config.cache_index as usize);
                let price = cache.oracle_price();
                let total_position =
                    fixed_to_ui(pos.derivative.base_position(), market_config.decimals);
                let position_value = total_position.checked_mul(price).unwrap();
                println!(
                    "| {:^5} | {:^10} | {:>25.4} | {:>25.4} | {:>25.4} | {:>15} | {:>15} | {:>15} | {:>15} |",
                    pos_idx,
                    market_name,
                    total_position,
                    price,
                    position_value,
                    native_to_ui(pos.derivative.open_orders_cache.coin_total, market.state.inner.config.decimals),
                    native_to_ui(pos.derivative.open_orders_cache.coin_total
                        - pos.derivative.open_orders_cache.coin_free, market.state.inner.config.decimals),
                    native_to_ui(pos.derivative.open_orders_cache.pc_total, QUOTE_TOKEN_DECIMALS),
                    native_to_ui(pos.derivative.open_orders_cache.pc_total - pos.derivative.open_orders_cache.pc_free, QUOTE_TOKEN_DECIMALS),
                );
            }
        }
    }
    let (maint_c_ratio, maint_assets, maint_liabilities) = sub_account
        .get_margin_c_ratio_components(
            cache_account.as_ref(),
            MarginCollateralRatioType::Maintenance,
        );
    let (init_c_ratio, init_assets, init_liabilities) = sub_account.get_margin_c_ratio_components(
        cache_account.as_ref(),
        MarginCollateralRatioType::Initialization,
    );
    println!(
        "\n\tMaint. Assets: {}\n\tMaint. Liabilities: {}\n\tMaint. C-Ratio: {}\n\tInit. Assets: {}\n\tInit. Liabilities: {}\n\tInit. C-Ratio: {}\n",
        maint_assets.separate_with_commas(),
        maint_liabilities.separate_with_commas(),
        maint_c_ratio,
        init_assets.separate_with_commas(),
        init_liabilities.separate_with_commas(),
        init_c_ratio,
    );

    Ok(CliResult {})
}
