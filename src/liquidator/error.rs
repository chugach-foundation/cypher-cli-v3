use anchor_lang::prelude::ProgramError;
use solana_client::client_error::ClientError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    ClientError(#[from] ClientError),
    #[error(transparent)]
    AnchorError(#[from] anchor_lang::error::Error),
    #[error(transparent)]
    ProgramError(#[from] ProgramError),
    #[error("Liquidator c-ratio below initialization.")]
    LiquidatorCRatio,
    #[error("An unrecognized account was received.")]
    UnrecognizedAccount,
}
