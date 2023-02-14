use log::{info, warn};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json;
use solana_client::{client_error::ClientError, nonblocking::rpc_client::RpcClient};
use solana_sdk::signature::Keypair;
use std::{fmt::Debug, fs::File, path::PathBuf, str::FromStr, sync::Arc};
use thiserror::Error;

use crate::{
    common::info::UserInfo,
    utils::accounts::{get_or_create_account, get_or_create_sub_account},
};

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("Unrecognized symbol: {0}")]
    UnrecognizedSymbol(String),
    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),
    #[error("Invalid path: {path:?}")]
    InvalidPath { path: String },
    #[error("File doesn't exist: {path:?}")]
    FileDoesntExist { path: PathBuf },
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config<T> {
    pub account_number: u8,
    pub sub_account_number: u8,
    pub inner: T,
}

/// Defines shared functionality for structs that should persist on the filesystem inside the config directory.
pub trait PersistentConfig<T>: DeserializeOwned + Serialize {
    /// Load from the given path.
    fn load(path: &str) -> Result<Self, ConfigError> {
        let path = match PathBuf::from_str(path) {
            Ok(p) => p,
            Err(_e) => {
                return Err(ConfigError::InvalidPath {
                    path: path.to_string(),
                })
            }
        };

        if path.exists() {
            log::info!(
                "Attempting to load persistent data from: {}",
                path.display()
            );
            let file = File::open(&path)?;

            Ok(serde_json::from_reader(&file)?)
        } else {
            Err(ConfigError::FileDoesntExist { path })
        }
    }
}

impl<T> PersistentConfig<T> for Config<T> where T: DeserializeOwned + Serialize {}

/// Gets the user's info required to
pub async fn get_user_info<T>(
    rpc_client: Arc<RpcClient>,
    config: &Config<T>,
    keypair: &Keypair,
) -> Result<UserInfo, ClientError> {
    info!("Preparing user info..");

    // we'll do this because we can't clone the keypair
    let signer = Arc::new(Keypair::from_bytes(&keypair.to_bytes()).unwrap());

    let (account_state, master_account) =
        match get_or_create_account(&rpc_client, &keypair, config.account_number).await {
            Ok(a) => a,
            Err(e) => {
                warn!(
                    "There was an error getting or creating Cypher account: {}",
                    e.to_string()
                );
                return Err(e);
            }
        };

    let (_sub_acccount_state, sub_account) = match get_or_create_sub_account(
        &rpc_client,
        &keypair,
        &master_account,
        config.sub_account_number,
    )
    .await
    {
        Ok(a) => a,
        Err(e) => {
            warn!(
                "There was an error getting or creating Cypher sub account: {}",
                e.to_string()
            );
            return Err(e);
        }
    };

    Ok(UserInfo {
        master_account,
        sub_account,
        clearing: account_state.clearing,
        signer,
    })
}
