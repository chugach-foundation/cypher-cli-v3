use async_trait::async_trait;
use thiserror::Error;

use super::{hedger::HedgerError, maker::MakerError};

#[derive(Error, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum StrategyError {
    #[error(transparent)]
    Maker(#[from] MakerError),
    #[error(transparent)]
    Hedger(#[from] HedgerError),
}

/// A trait that represents shared functionality for strategies.
#[async_trait]
pub trait Strategy: Send + Sync {
    /// The contextinput for the [`Strategy`].
    type Input;

    /// The output type of the [`Strategy`]'s execution
    type Output;

    /// Executes the [`Strategy`]].
    async fn execute(&self, ctx: &Self::Input) -> Result<Self::Output, StrategyError>;
}
