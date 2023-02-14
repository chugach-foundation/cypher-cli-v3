use solana_client::client_error::ClientError;
use thiserror::Error;

use crate::config::ConfigError;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    InvalidConfig(ConfigError),
    #[error(transparent)]
    ClientError(#[from] ClientError),
    #[error(transparent)]
    AnchorError(#[from] anchor_lang::error::Error),
    #[error("Liquidator c-ratio below initialization.")]
    LiquidatorCRatio,
    #[error("An unrecognized account was received.")]
    UnrecognizedAccount,
}
