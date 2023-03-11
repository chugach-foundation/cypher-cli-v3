use async_trait::async_trait;
use cypher_utils::contexts::Order;
use log::info;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::signature::Keypair;
use std::sync::Arc;
use tokio::sync::{
    broadcast::{Receiver, Sender},
    RwLock, RwLockReadGuard, RwLockWriteGuard,
};

use crate::common::{
    context::{ExecutionContext, GlobalContext},
    hedger::{Hedger, HedgerError, HedgerPulseResult},
    info::{MarketMetadata, PerpMarketInfo, UserInfo},
    inventory::InventoryManager,
    orders::{InflightCancel, ManagedOrder},
    Identifier, order_manager::OrderManager,
};

pub struct PerpsHedger {
    rpc_client: Arc<RpcClient>,
    signer: Arc<Keypair>,
    inventory_mngr: Arc<dyn InventoryManager<Input = GlobalContext>>,
    managed_orders: RwLock<Vec<ManagedOrder>>,
    open_orders: RwLock<Vec<Order>>,
    inflight_orders: RwLock<Vec<ManagedOrder>>,
    inflight_cancels: RwLock<Vec<InflightCancel>>,
    client_order_id: RwLock<u64>,
    shutdown_sender: Arc<Sender<bool>>,
    context_sender: Arc<Sender<ExecutionContext>>,
    user_info: UserInfo,
    market_info: PerpMarketInfo,
    market_metadata: MarketMetadata,
    symbol: String,
}

impl PerpsHedger {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        rpc_client: Arc<RpcClient>,
        signer: Arc<Keypair>,
        inventory_mngr: Arc<dyn InventoryManager<Input = GlobalContext>>,
        shutdown_sender: Arc<Sender<bool>>,
        context_sender: Arc<Sender<ExecutionContext>>,
        user_info: UserInfo,
        market_info: PerpMarketInfo,
        market_metadata: MarketMetadata,
        symbol: String,
    ) -> Self {
        Self {
            rpc_client,
            signer,
            inventory_mngr,
            symbol,
            shutdown_sender,
            context_sender,
            user_info,
            market_info,
            market_metadata,
            client_order_id: RwLock::new(u64::default()),
            managed_orders: RwLock::new(Vec::new()),
            open_orders: RwLock::new(Vec::new()),
            inflight_orders: RwLock::new(Vec::new()),
            inflight_cancels: RwLock::new(Vec::new()),
        }
    }
}

impl Identifier for PerpsHedger {
    fn symbol(&self) -> &str {
        self.symbol.as_str()
    }
}

#[async_trait]
impl OrderManager for PerpsHedger {
    async fn managed_orders_reader(&self) -> RwLockReadGuard<Vec<ManagedOrder>> {
        self.managed_orders.read().await
    }

    async fn managed_orders_writer(&self) -> RwLockWriteGuard<Vec<ManagedOrder>> {
        self.managed_orders.write().await
    }

    async fn open_orders_reader(&self) -> RwLockReadGuard<Vec<Order>> {
        self.open_orders.read().await
    }

    async fn open_orders_writer(&self) -> RwLockWriteGuard<Vec<Order>> {
        self.open_orders.write().await
    }

    async fn inflight_orders_reader(&self) -> RwLockReadGuard<Vec<ManagedOrder>> {
        self.inflight_orders.read().await
    }

    async fn inflight_orders_writer(&self) -> RwLockWriteGuard<Vec<ManagedOrder>> {
        self.inflight_orders.write().await
    }

    async fn inflight_cancels_reader(&self) -> RwLockReadGuard<Vec<InflightCancel>> {
        self.inflight_cancels.read().await
    }

    async fn inflight_cancels_writer(&self) -> RwLockWriteGuard<Vec<InflightCancel>> {
        self.inflight_cancels.write().await
    }
}

#[async_trait]
impl Hedger for PerpsHedger
where
    PerpsHedger: Identifier,
{
    type Input = ExecutionContext;
    type InventoryManagerInput = GlobalContext;

    fn rpc_client(&self) -> Arc<RpcClient> {
        self.rpc_client.clone()
    }

    fn signer(&self) -> Arc<Keypair> {
        self.signer.clone()
    }

    fn inventory_manager(&self) -> Arc<dyn InventoryManager<Input = GlobalContext>> {
        self.inventory_mngr.clone()
    }

    fn context_receiver(&self) -> Receiver<ExecutionContext> {
        self.context_sender.subscribe()
    }

    fn shutdown_receiver(&self) -> Receiver<bool> {
        self.shutdown_sender.subscribe()
    }

    async fn pulse(&self, _input: &Self::Input) -> Result<HedgerPulseResult, HedgerError> {
        info!("[{}] Running cypher perps hedger logic..", self.symbol);

        Ok(HedgerPulseResult::default())
    }
}
