use async_trait::async_trait;
use cypher_client::constants::QUOTE_TOKEN_DECIMALS;
use cypher_client::instructions::{cancel_spot_order, new_spot_order};
use cypher_client::utils::{convert_coin_to_lots, convert_price_to_lots};
use cypher_client::{
    cache_account, CancelOrderArgs, NewSpotOrderArgs, OrderType, SelfTradeBehavior, Side,
};
use cypher_utils::contexts::Order;
use fixed::types::I80F48;
use log::info;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::instruction::Instruction;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use std::any::type_name;
use std::ops::Mul;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast::{Receiver, Sender};
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::common::context::{ExecutionContext, GlobalContext, OperationContext};
use crate::common::info::{MarketMetadata, SpotMarketInfo, UserInfo};
use crate::common::inventory::InventoryManager;
use crate::common::maker::{Maker, MakerError, MakerPulseResult};
use crate::common::oracle::OracleInfo;
use crate::common::orders::{
    CandidateCancel, CandidatePlacement, InflightCancel, ManagedOrder, OrdersInfo,
};
use crate::common::strategy::{Strategy, StrategyError};

pub struct SpotMaker {
    rpc_client: Arc<RpcClient>,
    signer: Arc<Keypair>,
    inventory_mngr: Arc<dyn InventoryManager<Input = GlobalContext>>,
    managed_orders: RwLock<Vec<ManagedOrder>>,
    open_orders: RwLock<Vec<Order>>,
    inflight_orders: RwLock<Vec<ManagedOrder>>,
    inflight_cancels: RwLock<Vec<InflightCancel>>,
    client_order_id: RwLock<u64>,
    context: RwLock<ExecutionContext>,
    shutdown_sender: Arc<Sender<bool>>,
    context_sender: Arc<Sender<ExecutionContext>>,
    user_info: UserInfo,
    market_info: SpotMarketInfo,
    market_metadata: MarketMetadata,
    order_layers: usize,
    layer_spacing: u16,
    time_in_force: u64,
    symbol: String,
}

impl SpotMaker {
    pub fn new(
        rpc_client: Arc<RpcClient>,
        signer: Arc<Keypair>,
        inventory_mngr: Arc<dyn InventoryManager<Input = GlobalContext>>,
        shutdown_sender: Arc<Sender<bool>>,
        context_sender: Arc<Sender<ExecutionContext>>,
        user_info: UserInfo,
        market_info: SpotMarketInfo,
        market_metadata: MarketMetadata,
        order_layers: usize,
        layer_spacing: u16,
        time_in_force: u64,
        symbol: String,
    ) -> Self {
        Self {
            rpc_client,
            signer,
            inventory_mngr,
            order_layers,
            layer_spacing,
            time_in_force,
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
            context: RwLock::new(ExecutionContext::default()),
        }
    }
}

#[async_trait]
impl Maker for SpotMaker {
    type Input = ExecutionContext;
    type InventoryManagerInput = GlobalContext;

    fn rpc_client(&self) -> Arc<RpcClient> {
        self.rpc_client.clone()
    }

    fn signer(&self) -> Arc<Keypair> {
        self.signer.clone()
    }

    fn order_layer_count(&self) -> usize {
        self.order_layers
    }

    fn layer_spacing_bps(&self) -> u16 {
        self.layer_spacing
    }

    fn time_in_force(&self) -> u64 {
        60 // let's simply default to 60
    }

    fn context_receiver(&self) -> Receiver<ExecutionContext> {
        self.context_sender.subscribe()
    }

    fn shutdown_receiver(&self) -> Receiver<bool> {
        self.shutdown_sender.subscribe()
    }

    async fn context_reader(&self) -> RwLockReadGuard<ExecutionContext> {
        self.context.read().await
    }

    async fn context_writer(&self) -> RwLockWriteGuard<ExecutionContext> {
        self.context.write().await
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

    async fn pulse(&self) -> Result<MakerPulseResult, MakerError> {
        let ctx = self.context_reader().await;
        let inventory_mngr = self.inventory_manager();

        info!(
            "{} - [{}] Oracle Source: {:?} - Price: {}",
            type_name::<Self>(),
            self.symbol,
            ctx.oracle_info.source,
            ctx.oracle_info.price,
        );

        let spread_info = inventory_mngr.get_spread(ctx.oracle_info.price);
        if spread_info.oracle_price == I80F48::ZERO {
            return Ok(MakerPulseResult::default());
        }
        info!(
            "{} - [{}] Mid Price: {} - Best Bid: {} - Best Ask: {}",
            type_name::<Self>(),
            self.symbol,
            spread_info.oracle_price,
            spread_info.bid,
            spread_info.ask,
        );

        let quote_volumes = inventory_mngr.get_quote_volumes(&ctx.global);
        info!(
            "{} - [{}] Current delta: {} - Volumes - Bid: {} - Ask: {}",
            type_name::<Self>(),
            self.symbol,
            quote_volumes.delta,
            quote_volumes.bid_size,
            quote_volumes.ask_size,
        );
        if quote_volumes.bid_size == I80F48::ZERO && quote_volumes.ask_size == I80F48::ZERO {
            return Ok(MakerPulseResult::default());
        }

        let orders = self.get_orders().await;

        self.update_orders(&quote_volumes, &spread_info, &orders)
            .await
    }

    fn inventory_manager(&self) -> Arc<dyn InventoryManager<Input = GlobalContext>> {
        self.inventory_mngr.clone()
    }

    async fn build_new_order_ixs(
        &self,
        order_placements: &[CandidatePlacement],
    ) -> (Vec<Instruction>, Vec<ManagedOrder>) {
        let mut client_order_id = self.client_order_id.write().await;
        let mut new_order_ixs = Vec::new();
        let mut managed_orders = Vec::new();

        let cur_ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();

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
            let max_coin_qty = convert_coin_to_lots(
                order_placement
                    .base_quantity
                    .mul(I80F48::from(
                        10u64.pow(self.market_metadata.decimals as u32),
                    ))
                    .to_num(),
                self.market_metadata.quote_multiplier,
            );
            let max_native_pc_qty_including_fees = limit_price * max_coin_qty;
            let vault_signer = match &order_placement.side {
                Side::Bid => &self.market_info.quote_vault_signer, // if the order is a bid, we need the quote vault signer
                Side::Ask => &self.market_info.asset_vault_signer, // if the order is an ask, we need the base vault signer
            };
            new_order_ixs.push(new_spot_order(
                &self.user_info.clearing,
                &cache_account::id(),
                &self.user_info.master_account,
                &self.user_info.sub_account,
                &self.market_info.asset_pool_nodes.first().unwrap(),
                &self.market_info.quote_pool_nodes.first().unwrap(),
                &self.market_info.asset_mint,
                &self.market_info.asset_vault,
                &self.market_info.quote_vault,
                vault_signer,
                &self.signer.pubkey(),
                &self.market_info.market,
                &self.market_info.open_orders,
                &self.market_info.event_queue,
                &self.market_info.request_queue,
                &self.market_info.bids,
                &self.market_info.asks,
                &self.market_info.dex_coin_vault,
                &self.market_info.dex_pc_vault,
                &self.market_info.dex_vault_signer,
                NewSpotOrderArgs {
                    side: order_placement.side,
                    limit_price,
                    max_coin_qty,
                    max_native_pc_qty_including_fees,
                    order_type: OrderType::PostOnly,
                    self_trade_behavior: SelfTradeBehavior::CancelProvide,
                    client_order_id: *client_order_id,
                    limit: u16::MAX,
                },
            ));
            managed_orders.push(ManagedOrder {
                price_lots: limit_price,
                base_quantity_lots: max_coin_qty,
                quote_quantity_lots: max_native_pc_qty_including_fees,
                price: order_placement.price,
                base_quantity: order_placement.base_quantity,
                max_quote_quantity: order_placement.max_quote_quantity,
                max_ts: u64::default(),
                submitted_at: cur_ts.as_secs(),
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
            let is_client_id = if order_cancel.order_id == u128::default() {
                true
            } else {
                false
            };
            let order_id = if is_client_id {
                order_cancel.client_order_id as u128
            } else {
                order_cancel.order_id
            };
            cancel_order_ixs.push(cancel_spot_order(
                &self.user_info.clearing,
                &cache_account::id(),
                &self.user_info.master_account,
                &self.user_info.sub_account,
                &self.market_info.asset_pool_nodes.first().unwrap(),
                &self.market_info.quote_pool_nodes.first().unwrap(),
                &self.market_info.asset_mint,
                &self.market_info.asset_vault,
                &self.market_info.quote_vault,
                &self.signer.pubkey(),
                &self.market_info.market,
                &self.market_info.open_orders,
                &self.market_info.event_queue,
                &self.market_info.bids,
                &self.market_info.asks,
                &self.market_info.dex_coin_vault,
                &self.market_info.dex_pc_vault,
                &self.market_info.dex_vault_signer,
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
