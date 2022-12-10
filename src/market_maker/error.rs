use solana_client::client_error::ClientError;
use thiserror::Error;

use super::config::ConfigError;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    InvalidConfig(ConfigError),
    #[error(transparent)]
    ClientError(#[from] ClientError),
}
