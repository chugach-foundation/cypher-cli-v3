use async_trait::async_trait;
use cypher_client::constants::QUOTE_TOKEN_DECIMALS;
use cypher_client::instructions::{cancel_perp_orders, multiple_new_perp_orders};
use cypher_client::utils::{convert_coin_to_lots, convert_price_to_lots};
use cypher_client::{cache_account, CancelOrderArgs, DerivativeOrderType, NewDerivativeOrderArgs};
use cypher_utils::contexts::Order;
use fixed::types::I80F48;
use log::info;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::instruction::Instruction;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use std::ops::Mul;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast::{Receiver, Sender};
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::common::context::{ExecutionContext, GlobalContext};
use crate::common::info::{MarketMetadata, PerpMarketInfo, UserInfo};
use crate::common::inventory::InventoryManager;
use crate::common::maker::{Maker, MakerError, MakerPulseResult};
use crate::common::order_manager::OrderManager;
use crate::common::orders::{CandidateCancel, CandidatePlacement, InflightCancel, ManagedOrder};
use crate::common::Identifier;

pub struct PerpsMaker {
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
    order_layers: usize,
    layer_spacing: u16,
    time_in_force: u64,
    symbol: String,
}

impl PerpsMaker {
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
        }
    }
}

impl Identifier for PerpsMaker {
    fn symbol(&self) -> &str {
        self.symbol.as_str()
    }
}

#[async_trait]
impl OrderManager for PerpsMaker {
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
impl Maker for PerpsMaker
where
    PerpsMaker: Identifier,
{
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
        self.time_in_force
    }

    fn context_receiver(&self) -> Receiver<ExecutionContext> {
        self.context_sender.subscribe()
    }

    fn shutdown_receiver(&self) -> Receiver<bool> {
        self.shutdown_sender.subscribe()
    }

    async fn pulse(&self, input: &<Self as Maker>::Input) -> Result<MakerPulseResult, MakerError> {
        let inventory_mngr = self.inventory_manager();

        let spread_info = inventory_mngr.get_spread(input.oracle_info.price);
        if spread_info.oracle_price == I80F48::ZERO {
            return Ok(MakerPulseResult::default());
        }

        let quote_volumes = inventory_mngr.get_quote_volumes(&input.global);
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
        let mut new_order_args = Vec::new();
        let mut managed_orders = Vec::new();

        let time_in_force = self.time_in_force();

        let cur_ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let max_ts = cur_ts.as_secs() + time_in_force;

        let mut idx = 0;

        for order_placement in order_placements.iter() {
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
            let max_base_qty = convert_coin_to_lots(
                order_placement
                    .base_quantity
                    .mul(I80F48::from(
                        10u64.pow(self.market_metadata.decimals as u32),
                    ))
                    .to_num(),
                self.market_metadata.base_multiplier,
            );
            let max_quote_qty = limit_price * max_base_qty;
            info!(
                "[{}] Placing order. Limit Price: {} - Limit Price (fp32): {} - Max Base Qty: {} - Max Quote Qty: {} - Client ID: {}",
                self.symbol(),
                limit_price,
                limit_price << 32,
                max_base_qty,
                max_quote_qty,
                *client_order_id
            );
            new_order_args.push(NewDerivativeOrderArgs {
                side: order_placement.side,
                limit_price,
                max_base_qty,
                max_quote_qty,
                order_type: DerivativeOrderType::PostOnly,
                client_order_id: *client_order_id,
                limit: u16::MAX,
                max_ts,
            });
            managed_orders.push(ManagedOrder {
                price_lots: limit_price,
                base_quantity_lots: max_base_qty,
                quote_quantity_lots: max_quote_qty,
                price: order_placement.price,
                base_quantity: order_placement.base_quantity,
                max_quote_quantity: order_placement.max_quote_quantity,
                max_ts,
                submitted_at: cur_ts.as_secs(),
                client_order_id: *client_order_id,
                order_id: u128::default(),
                side: order_placement.side,
                layer: order_placement.layer,
            });
            *client_order_id += 1;

            // this method is good to place roughly 15 orders with one instruction
            // so let's be conservtive and cap it at ~12 in case we need to pass other instructions before
            if idx >= 11 {
                new_order_ixs.push(multiple_new_perp_orders(
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
                    self.market_info.quote_pool_nodes.first().unwrap(), // TODO: this should be done differently
                    &self.signer.pubkey(),
                    new_order_args.clone(),
                ));
                new_order_args.clear();
                idx = 0;
            }
            idx += 1;
        }

        // check if there are args but no ix was added
        if new_order_ixs.is_empty() && !new_order_args.is_empty() {
            new_order_ixs.push(multiple_new_perp_orders(
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
                self.market_info.quote_pool_nodes.first().unwrap(), // TODO: this should be done differently
                &self.signer.pubkey(),
                new_order_args,
            ));
        }

        (new_order_ixs, managed_orders)
    }

    async fn build_cancel_order_ixs(&self, order_cancels: &[CandidateCancel]) -> Vec<Instruction> {
        let mut cancel_order_ixs = Vec::new();
        let mut cancel_order_args = Vec::new();
        let mut idx = 0;

        for order_cancel in order_cancels.iter() {
            let is_client_id = order_cancel.order_id == u128::default();
            let order_id = if is_client_id {
                order_cancel.client_order_id as u128
            } else {
                order_cancel.order_id
            };
            info!(
                "[{}] Cancelling order. Is client ID: {} - ID: {}",
                self.symbol(),
                is_client_id,
                order_id,
            );
            cancel_order_args.push(CancelOrderArgs {
                order_id,
                side: order_cancel.side,
                is_client_id,
            });
            // this method is good to cancel around 30 orders with one instruction
            // but let's be conservative in case more tx space is needed before/after
            if idx >= 20 {
                cancel_order_ixs.push(cancel_perp_orders(
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
                    self.market_info.quote_pool_nodes.first().unwrap(), // TODO: this should be done differently
                    &self.signer.pubkey(),
                    cancel_order_args.clone(),
                ));
                cancel_order_args.clear();
                idx = 0;
            }
            idx += 1;
        }

        // check if there are args but no ix was added
        if cancel_order_ixs.is_empty() && !cancel_order_args.is_empty() {
            cancel_order_ixs.push(cancel_perp_orders(
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
                self.market_info.quote_pool_nodes.first().unwrap(), // TODO: this should be done differently
                &self.signer.pubkey(),
                cancel_order_args,
            ));
        }

        cancel_order_ixs
    }
}
