use clap::ArgMatches;
use cypher_utils::utils::load_keypair;
use solana_clap_utils::input_validators::{is_url_or_moniker, normalize_to_url_if_moniker};
use solana_client::nonblocking::{pubsub_client::PubsubClient, rpc_client::RpcClient};
use solana_sdk::{commitment_config::CommitmentConfig, signer::Signer};
use std::{error, sync::Arc};

use crate::cli::{command::parse_command, CliError};

use super::CliConfig;

pub async fn parse_args(matches: &ArgMatches<'_>) -> Result<CliConfig, Box<dyn error::Error>> {
    let json_rpc_url = matches.value_of("json_rpc_url").unwrap();
    let pubsub_rpc_url = matches.value_of("pubsub_rpc_url");
    let keypair_path = matches.value_of("keypair").unwrap();

    let (_is_moniker, _is_valid_url) = match is_url_or_moniker(json_rpc_url) {
        Ok(_) => (false, true),
        Err(_) => match json_rpc_url {
            "m" | "mainnet-beta" => (true, true),
            "t" | "testnet" => (true, true),
            "d" | "devnet" => (true, true),
            "l" | "localhost" => (true, true),
            _ => (false, false),
        },
    };

    let normalized_url = normalize_to_url_if_moniker(json_rpc_url);
    println!("Using JSON RPC API URL: {}", normalized_url);

    println!("Loading keypair from: {}", keypair_path);
    let keypair = match load_keypair(keypair_path) {
        Ok(k) => k,
        Err(e) => {
            return Err(Box::new(CliError::KeypairError(e)));
        }
    };

    println!("Loaded keypair with address: {}", keypair.pubkey());
    let command = parse_command(matches)?;

    let (pubsub_client, pubsub_rpc_url) = if let Some(pubsub_url) = pubsub_rpc_url {
        println!("Using PubSub RPC WS URL: {}", pubsub_url);
        match PubsubClient::new(pubsub_url).await {
            Ok(pc) => (Some(Arc::new(pc)), "".to_string()),
            Err(_e) => {
                // do not throw an error just yet as the pubsub client might not actually be needed
                (None, "".to_string())
            }
        }
    } else {
        (None, "".to_string())
    };

    Ok(CliConfig {
        command,
        json_rpc_url: normalized_url.to_string(),
        pubsub_rpc_url,
        keypair_path: keypair_path.to_string(),
        rpc_client: Some(Arc::new(RpcClient::new_with_commitment(
            normalized_url.to_string(),
            CommitmentConfig::processed(),
        ))),
        pubsub_client,
        keypair: Some(keypair),
    })
}
