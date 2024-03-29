use std::sync::Arc;

use async_trait::async_trait;
use fixed::types::I80F48;
use log::{info, warn};
use thiserror::Error;
use tokio::sync::broadcast::{error::SendError, Receiver, Sender};

use super::Identifier;

#[derive(Error, Debug)]
pub enum OracleProviderError {
    #[error("Send error: {:?}", self)]
    OutputSendError(SendError<OracleInfo>),
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
    /// Decentralized exchange.
    Dex(DecentralizedExchangeSource),
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
    /// Timestamp of when this price feed was recorded, in milliseconds.
    pub timestamp: u128,
}

/// A trait that represents shared functionality for oracle providers.
///
/// This trait can be implemented to offer different execution environments for strategies.
#[async_trait]
pub trait OracleProvider: Send + Sync + Identifier {
    /// The input data of the [`OracleProvider`].
    type Input: Send + Sync + Clone;

    /// Gets the [`OracleProvider`]'s input receiver.
    fn input_receiver(&self) -> Receiver<Self::Input>;

    /// Gets the shutdown [`Receiver`].
    fn shutdown_receiver(&self) -> Receiver<bool>;

    /// Starts the [`OracleProvider`],
    async fn start(&self) -> Result<(), OracleProviderError> {
        let mut input_receiver = self.input_receiver();
        let mut shutdown_receiver = self.shutdown_receiver();

        info!("[{}] Starting oracle provider..", self.symbol());

        loop {
            tokio::select! {
                input_update = input_receiver.recv() => {
                    match input_update {
                        Ok(input) => {
                            match self.process_update(&input) {
                                Ok(output) => {
                                    info!("[{}] Oracle Info: {:?}", self.symbol(), output);
                                    match self.send(output).await {
                                        Ok(r) => info!("[{}] Successfully sent context update to {} receivers.", self.symbol(), r),
                                        Err(e) => {
                                            warn!("[{}] There was an error sending oracle price update. Error: {:?}", self.symbol(), e);
                                        }
                                    };
                                },
                                Err(e) => {
                                    warn!("[{}] There was an error processing account update. Error: {:?}", self.symbol(), e);
                                }
                            };
                        },
                        Err(e) => {
                            warn!("[{}] There was an error receiving account state update. Error: {:?}", self.symbol(), e);
                        }
                    }
                }
                _ = shutdown_receiver.recv() => {
                    info!("[{}] Shutdown signal received, stopping..", self.symbol());
                    break;
                }
            }
        }

        Ok(())
    }

    /// Processes a [`Self::Input`] update from the input provider.
    fn process_update(&self, input: &Self::Input) -> Result<OracleInfo, OracleProviderError>;

    /// Gets the output [`Sender`] of the [`OracleProvider`].
    fn output_sender(&self) -> Arc<Sender<OracleInfo>>;

    /// Sends an update to oracle price feed subscribers via it's own [`Sender`].
    async fn send(&self, output: OracleInfo) -> Result<usize, OracleProviderError> {
        let sender = self.output_sender();
        match sender.send(output) {
            Ok(r) => Ok(r), // we can safely ignore this result
            Err(e) => Err(OracleProviderError::OutputSendError(e)),
        }
    }

    /// Subscribes to the [`OracleProvider`].
    fn subscribe(&self) -> Receiver<OracleInfo>;
}
