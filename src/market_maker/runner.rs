use anchor_spl::dex::serum_dex::state::Market;
use cypher_utils::{accounts_cache::AccountsCache, services::StreamingAccountInfoService};
use solana_client::nonblocking::{pubsub_client::PubsubClient, rpc_client::RpcClient};
use std::{sync::Arc, time::Duration};
use tokio::sync::{broadcast::Sender, RwLock};

use super::{config::Config, Error};

pub struct Runner {
    config: Arc<Config>,
    rpc_client: Arc<RpcClient>,
    pubsub_client: Arc<PubsubClient>,
    accounts_cache: Arc<AccountsCache>,
    streaming_service: Arc<StreamingAccountInfoService>,
    shutdown: Arc<Sender<bool>>,
}

impl Runner {
    pub async fn new(
        config: Arc<Config>,
        rpc_client: Arc<RpcClient>,
        pubsub_client: Arc<PubsubClient>,
        shutdown: Arc<Sender<bool>>,
    ) -> Self {
        Self {
            config,
            rpc_client,
            pubsub_client,
            shutdown,
            accounts_cache: Arc::new(AccountsCache::new()),
            streaming_service: Arc::new(StreamingAccountInfoService::default().await),
        }
    }

    pub async fn run(self) -> Result<(), Error> {
        let mut i = 0;
        loop {
            log::info!(
                "[runner] running in a loop doing market maker things, {}",
                i
            );
            i += 1;
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        Ok(())
    }
}
