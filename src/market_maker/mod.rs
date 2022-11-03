use thiserror::Error;

pub mod config;
mod constants;
mod context;
mod executor;
mod hedging;
mod inventory;
mod making;
pub mod runner;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Default error.")]
    Default,
}
