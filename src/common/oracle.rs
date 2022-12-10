use async_trait::async_trait;
use fixed::types::I80F48;
use thiserror::Error;
use tokio::sync::broadcast::Receiver;

#[derive(Error, Debug)]
pub enum OracleProviderError {}

/// Represents centralized exchange based oracle sources.
#[derive(Debug, Clone)]
pub enum CentralizedExchangeSource {
    Binance,
}

impl Default for CentralizedExchangeSource {
    fn default() -> Self {
        Self::Binance
    }
}

/// Represents decentralized exchange based oracle sources.
#[derive(Debug, Clone)]
pub enum DecentralizedExchangeSource {
    Cypher,
}

impl Default for DecentralizedExchangeSource {
    fn default() -> Self {
        Self::Cypher
    }
}

/// Represents on-chain oracle sources.
#[derive(Debug, Clone)]
pub enum OnchainOracleSource {
    Pyth,
}

impl Default for OnchainOracleSource {
    fn default() -> Self {
        Self::Pyth
    }
}

/// Represents the source type of the [`OracleInfo`] source.
#[derive(Debug, Clone)]
pub enum OracleInfoSource {
    /// Centralized exchange.
    Cex(CentralizedExchangeSource),
    /// Decentralized exchange.
    Dex(CentralizedExchangeSource),
    /// On-chain.
    Onchain(OnchainOracleSource),
}

impl Default for OracleInfoSource {
    fn default() -> Self {
        Self::Onchain(OnchainOracleSource::Pyth)
    }
}

/// Represents an oracle price feed,
#[derive(Debug, Default, Clone)]
pub struct OracleInfo {
    /// The symbol of the underlying asset this price feed represents.
    pub symbol: String,
    /// The source of this oracle info.
    pub source: OracleInfoSource,
    /// The price of the asset.
    pub price: I80F48,
    /// Timestamp of when this price feed was recorded.
    pub timestamp: u64,
}

/// A trait that represents shared functionality for oracle providers.
///
/// This trait can be implemented to offer different execution environments for strategies.
#[async_trait]
pub trait OracleProvider: Send + Sync {
    fn new() -> Self;

    /// Starts the [`OracleProvider`],
    async fn start(&self) -> Result<(), OracleProviderError>;

    /// Subscribes to the [`OracleProvider`].
    fn subscribe(&self) -> Receiver<OracleInfo>;
}
