use async_trait::async_trait;
use cypher_client::{
    cache_account,
    constants::QUOTE_TOKEN_DECIMALS,
    instructions::{cancel_perp_order, new_perp_order},
    utils::convert_price_to_lots,
    CancelOrderArgs, DerivativeOrderType, NewDerivativeOrderArgs, SelfTradeBehavior,
};
use cypher_utils::contexts::Order;
use fixed::types::I80F48;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    instruction::Instruction,
    signature::{Keypair, Signature},
    signer::Signer,
};
use std::{
    ops::Mul,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::{
    broadcast::{Receiver, Sender},
    RwLock, RwLockReadGuard, RwLockWriteGuard,
};

use crate::common::{
    context::OperationContext,
    info::{MarketMetadata, PerpMarketInfo, UserInfo},
    orders::{
        CandidateCancel, CandidatePlacement, InflightCancel, ManagedOrder, OrderManager,
        OrderManagerError,
    },
};

pub struct PerpsOrderManager {
    rpc_client: Arc<RpcClient>,
    signer: Arc<Keypair>,
    shutdown_sender: Arc<Sender<bool>>,
    context_sender: Arc<Sender<OperationContext>>,
    client_order_id: RwLock<u64>,
    managed_orders: RwLock<Vec<ManagedOrder>>,
    open_orders: RwLock<Vec<Order>>,
    inflight_orders: RwLock<Vec<ManagedOrder>>,
    inflight_cancels: RwLock<Vec<InflightCancel>>,
    user_info: UserInfo,
    market_info: PerpMarketInfo,
    market_metadata: MarketMetadata,
    symbol: String,
}

impl PerpsOrderManager {
    pub fn new(
        rpc_client: Arc<RpcClient>,
        signer: Arc<Keypair>,
        shutdown_sender: Arc<Sender<bool>>,
        context_sender: Arc<Sender<OperationContext>>,
        user_info: UserInfo,
        market_info: PerpMarketInfo,
        market_metadata: MarketMetadata,
        symbol: String,
    ) -> Self {
        Self {
            rpc_client,
            signer,
            shutdown_sender,
            context_sender,
            user_info,
            market_info,
            market_metadata,
            symbol,
            client_order_id: RwLock::new(u64::default()),
            managed_orders: RwLock::new(Vec::new()),
            open_orders: RwLock::new(Vec::new()),
            inflight_orders: RwLock::new(Vec::new()),
            inflight_cancels: RwLock::new(Vec::new()),
        }
    }
}

#[async_trait]
impl OrderManager for PerpsOrderManager {
    type Input = OperationContext;

    fn rpc_client(&self) -> Arc<RpcClient> {
        self.rpc_client.clone()
    }

    fn signer(&self) -> Arc<Keypair> {
        self.signer.clone()
    }

    fn context_receiver(&self) -> Receiver<OperationContext> {
        self.context_sender.subscribe()
    }

    fn shutdown_receiver(&self) -> Receiver<bool> {
        self.shutdown_sender.subscribe()
    }

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

    async fn build_new_order_ixs(
        &self,
        order_placements: &[CandidatePlacement],
    ) -> (Vec<Instruction>, Vec<ManagedOrder>) {
        let mut client_order_id = self.client_order_id.write().await;
        let mut new_order_ixs = Vec::new();
        let mut managed_orders = Vec::new();

        let cur_ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let max_ts = cur_ts.as_secs() + 60;

        for order_placement in order_placements {
            // OBS: this might be subject to changes, there's more efficient ways to do this obviously :)
            let limit_price = convert_price_to_lots(
                order_placement
                    .price
                    .mul(I80F48::from(10u64.pow(QUOTE_TOKEN_DECIMALS as u32)))
                    .to_num(),
                self.market_metadata.base_multiplier,
                10u64.pow(self.market_metadata.decimals as u32),
                self.market_metadata.quote_multiplier,
            );
            let max_base_qty = order_placement
                .base_quantity
                .mul(I80F48::from(
                    10u64.pow(self.market_metadata.decimals as u32),
                ))
                .to_num();
            let max_quote_qty = limit_price * max_base_qty;

            new_order_ixs.push(new_perp_order(
                &self.user_info.clearing,
                &cache_account::id(),
                &self.user_info.master_account,
                &self.user_info.sub_account,
                &self.market_info.market,
                &self.market_info.orders,
                &self.market_info.orderbook,
                &self.market_info.event_queue,
                &self.market_info.bids,
                &self.market_info.asks,
                &self.market_info.quote_pool_nodes.first().unwrap(), // TODO: this should be done differently
                &self.signer.pubkey(),
                NewDerivativeOrderArgs {
                    side: order_placement.side,
                    limit_price,
                    max_base_qty,
                    max_quote_qty,
                    order_type: DerivativeOrderType::PostOnly,
                    self_trade_behavior: SelfTradeBehavior::AbortTransaction,
                    client_order_id: *client_order_id,
                    limit: u16::MAX,
                    max_ts,
                },
            ));
            managed_orders.push(ManagedOrder {
                price_lots: limit_price,
                base_quantity_lots: max_base_qty,
                quote_quantity_lots: max_quote_qty,
                price: order_placement.price,
                base_quantity: order_placement.base_quantity,
                max_quote_quantity: order_placement.max_quote_quantity,
                max_ts,
                client_order_id: *client_order_id,
                order_id: u128::default(),
                side: order_placement.side,
                layer: order_placement.layer,
            });
            *client_order_id += 1;
        }

        (new_order_ixs, managed_orders)
    }

    async fn build_cancel_order_ixs(&self, order_cancels: &[CandidateCancel]) -> Vec<Instruction> {
        let mut cancel_order_ixs = Vec::new();

        for order_cancel in order_cancels {
            let is_client_id = if order_cancel.client_order_id != u64::default() {
                true
            } else {
                false
            };
            let order_id = if is_client_id {
                order_cancel.client_order_id as u128
            } else {
                order_cancel.order_id
            };
            cancel_order_ixs.push(cancel_perp_order(
                &self.user_info.clearing,
                &cache_account::id(),
                &self.user_info.master_account,
                &self.user_info.sub_account,
                &self.market_info.market,
                &self.market_info.orders,
                &self.market_info.orderbook,
                &self.market_info.event_queue,
                &self.market_info.bids,
                &self.market_info.asks,
                &self.market_info.quote_pool_nodes.first().unwrap(), // TODO: this should be done differently
                &self.signer.pubkey(),
                CancelOrderArgs {
                    order_id,
                    side: order_cancel.side,
                    is_client_id,
                },
            ))
        }
        cancel_order_ixs
    }

    fn symbol(&self) -> &str {
        self.symbol.as_str()
    }
}
