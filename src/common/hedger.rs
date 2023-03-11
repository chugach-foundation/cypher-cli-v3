use async_trait::async_trait;
use log::{info, warn};
use solana_client::{client_error::ClientError, nonblocking::rpc_client::RpcClient};
use solana_sdk::signature::Keypair;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::broadcast::Receiver;

use super::{
    context::OrdersContext, inventory::InventoryManager, order_manager::OrderManager, Identifier,
};

/// Represents the result of an hedger's pulse.
#[derive(Default, Debug, Clone)]
pub struct HedgerPulseResult {
    /// The size executed during the pulse.
    /// Negative value means sell size executed, positive value means buy size executed.
    pub size_executed: i128,
}

#[derive(Debug, Error)]
pub enum HedgerError {
    #[error(transparent)]
    Client(#[from] ClientError),
}

/// Defines shared functionality that different hedgers should implement.
#[async_trait]
pub trait Hedger: Send + Sync + OrderManager + Identifier {
    /// The input type for the hedger.
    type Input: Clone + Send + Sync + OrdersContext;

    /// The input type for the [`InventoryManager`].
    type InventoryManagerInput: Clone + Send + Sync;

    /// Gets the [`RpcClient`].
    ///
    /// OBS: If there is a desire to turn this entire thing somewhat agnostic, might be worthwhile
    /// switching this for an `ExchangeAdapter` trait which abstracts away the transaction building and submission.
    fn rpc_client(&self) -> Arc<RpcClient>;

    /// Gets the [`Keypair`].
    ///
    /// OBS: If there is a desire to turn this entire thing somewhat agnostic, might be worthwhile
    /// switching this for an `ExchangeAdapter` trait which abstracts away the transaction building and submission.
    fn signer(&self) -> Arc<Keypair>;

    /// Gets the inventory manager for the [`Maker`].
    fn inventory_manager(&self) -> Arc<dyn InventoryManager<Input = Self::InventoryManagerInput>>;

    /// Gets the [`Receiver`] for the respective [`Self::Input`] data type.
    fn context_receiver(&self) -> Receiver<Self::Input>;

    /// Gets the [`Receiver`] for the existing orders.
    fn shutdown_receiver(&self) -> Receiver<bool>;

    /// Starts the [`OrderManager`],
    async fn start(&self) -> Result<(), HedgerError> {
        let mut context_receiver = self.context_receiver();
        let mut shutdown_receiver = self.shutdown_receiver();
        let symbol = self.symbol();

        info!("[{}] Starting maker..", symbol);

        loop {
            tokio::select! {
                ctx_update = context_receiver.recv() => {
                    match ctx_update {
                        Ok(ctx) => {
                            self.process_update(&ctx).await;
                            match self.pulse(&ctx).await {
                                Ok(res) => {
                                    info!("[{}] Hedger pulse: {:?}",  symbol, res);
                                },
                                Err(e) => {
                                    warn!("[{}] There was an error processing hedger pulse. Error: {:?}", symbol, e);
                                }
                            };
                        },
                        Err(e) => {
                            warn!("[{}] There was an error receiving hedger input context update. Error: {:?}", symbol, e);
                        }
                    }
                }
                _ = shutdown_receiver.recv() => {
                    info!("[{}] Shutdown signal received, stopping..", symbol);
                    break;
                }
            }
        }

        Ok(())
    }

    /// Triggers a pulse, prompting the [`Hedger`] to perform it's work cycle.
    /// This is also where any logic that needs to be performed before the generic [`Hedger`] work should be performed.
    async fn pulse(&self, input: &Self::Input) -> Result<HedgerPulseResult, HedgerError>;
}
