use {
    self::command::CliCommand,
    crate::market_maker::error::Error as MarketMakerError,
    cypher_utils::{contexts::ContextError, utils::KeypairError},
    solana_client::{
        client_error::ClientError,
        nonblocking::{pubsub_client::PubsubClient, rpc_client::RpcClient},
    },
    solana_sdk::signature::Keypair,
    std::sync::Arc,
    thiserror::Error,
};

pub mod account;
pub mod app;
pub mod args;
pub mod command;
pub mod faucet;
pub mod futures;
pub mod liquidator;
pub mod list;
pub mod market_maker;
pub mod perpetuals;
pub mod random;
pub mod spot;
pub mod sub_account;

pub use app::*;

/// The config of the CLI.
/// This structure holds all necessary information to process and execute a command.
pub struct CliConfig {
    /// The command we are processing.
    pub command: CliCommand,
    /// The URL for the JSON RPC API we are using.
    /// See: https://docs.solana.com/developing/clients/jsonrpc-api#json-rpc-api-reference
    pub json_rpc_url: String,
    /// The URL for the Streaming RPC we are using
    /// See: https://docs.solana.com/developing/clients/jsonrpc-api#subscription-websocket
    pub pubsub_rpc_url: String,
    /// The path to the keypair being used.
    pub keypair_path: String,
    /// The initialized RPC Client, possibly.
    pub rpc_client: Option<Arc<RpcClient>>,
    /// The initialized Pubsub Client, possibly.
    pub pubsub_client: Option<Arc<PubsubClient>>,
    /// The loaded keypair, possibly.
    pub keypair: Option<Keypair>,
}

/// The result of a CLI command.
/// This structure may hold generic or common data that pertains to the result of a CLI command.
/// It may even hold intermediate errors from a command's execution that did not end up interfering with the final result.
#[derive(Debug)]
pub struct CliResult {}

#[derive(Debug, Error)]
pub enum CliError {
    #[error("Bad parameter: {0}")]
    BadParameters(String),
    #[error(transparent)]
    ClientError(#[from] ClientError),
    #[error(transparent)]
    ContextError(#[from] ContextError),
    #[error("Command not recognized: {0}")]
    CommandNotRecognized(String),
    #[error("Keypair error: {:?}", self)]
    KeypairError(KeypairError),
    #[error("Market maker error: {:?}", self)]
    MarketMaker(MarketMakerError),
}
