use log::info;
use std::{
    any::type_name,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::broadcast::{channel, Receiver, Sender};

use crate::common::{
    context::{builder::ContextBuilder, GlobalContext},
    oracle::{
        DecentralizedExchangeSource, OracleInfo, OracleInfoSource, OracleProvider,
        OracleProviderError,
    },
};

pub struct CypherOracleProvider {
    global_context_builder: Arc<dyn ContextBuilder<Output = GlobalContext>>,
    shutdown_sender: Arc<Sender<bool>>,
    output_sender: Arc<Sender<OracleInfo>>,
    cache_index: usize,
    symbol: String,
}

impl CypherOracleProvider {
    pub fn new(
        global_context_builder: Arc<dyn ContextBuilder<Output = GlobalContext>>,
        shutdown_sender: Arc<Sender<bool>>,
        cache_index: usize,
        symbol: String,
    ) -> Self {
        Self {
            global_context_builder,
            shutdown_sender,
            output_sender: Arc::new(channel::<OracleInfo>(10).0),
            cache_index,
            symbol,
        }
    }
}

impl OracleProvider for CypherOracleProvider {
    type Input = GlobalContext;

    fn input_receiver(&self) -> Receiver<Self::Input> {
        self.global_context_builder.subscribe()
    }

    fn shutdown_receiver(&self) -> Receiver<bool> {
        self.shutdown_sender.subscribe()
    }

    fn process_update(&self, input: &GlobalContext) -> Result<OracleInfo, OracleProviderError> {
        let cache_ctx = &input.cache;

        let cache = cache_ctx.state.get_price_cache(self.cache_index);

        info!(
            "{} - Oracle price from cache at index {}: {}",
            type_name::<Self>(),
            self.cache_index,
            cache.oracle_price()
        );

        Ok(OracleInfo {
            symbol: self.symbol.to_string(),
            source: OracleInfoSource::Dex(DecentralizedExchangeSource::Cypher),
            price: cache.oracle_price(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        })
    }

    fn output_sender(&self) -> Arc<Sender<OracleInfo>> {
        self.output_sender.clone()
    }

    fn subscribe(&self) -> Receiver<OracleInfo> {
        self.output_sender.subscribe()
    }

    fn symbol(&self) -> &str {
        self.symbol.as_str()
    }
}
