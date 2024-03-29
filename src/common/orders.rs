use cypher_client::Side;

use fixed::types::I80F48;

#[derive(Default, Debug, Clone)]
pub struct CandidateCancel {
    /// The order's id.
    pub order_id: u128,
    /// The order's client id.
    pub client_order_id: u64,
    /// The order's side.
    pub side: Side,
    /// The layer that this order represents.
    pub layer: usize,
}

#[derive(Default, Debug, Clone)]
pub struct CandidatePlacement {
    /// The order's price.
    pub price: I80F48,
    /// The order's base quantity.
    pub base_quantity: I80F48,
    /// The order's max quote quantity.
    pub max_quote_quantity: I80F48,
    /// The order's time in force timestamp.
    pub max_ts: u64,
    /// The order's client id.
    pub client_order_id: u64,
    /// The order's side.
    pub side: Side,
    /// The layer that this order represents.
    pub layer: usize,
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct InflightCancel {
    /// The order id.
    pub order_id: u128,
    /// The client order id.
    pub client_order_id: u64,
    /// The order side.
    pub side: Side,
    /// The cancel's transaction submission timestamp.
    pub submitted_at: u64,
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct ManagedOrder {
    /// The order's price.
    pub price_lots: u64,
    /// The order's max base quantity.
    pub base_quantity_lots: u64,
    /// The order's max quote quantity.
    pub quote_quantity_lots: u64,
    /// The order's price.
    pub price: I80F48,
    /// The order's base quantity.
    pub base_quantity: I80F48,
    /// The order's max quote quantity.
    pub max_quote_quantity: I80F48,
    /// The order's time in force timestamp.
    pub max_ts: u64,
    /// The order's transaction submission timestamp.
    pub submitted_at: u64,
    /// The order's client id.
    pub client_order_id: u64,
    /// The order's id.
    pub order_id: u128,
    /// The side of the order.
    pub side: Side,
    /// The layer of the order.
    pub layer: usize,
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct OrdersInfo {
    pub open_orders: Vec<ManagedOrder>,
    pub inflight_cancels: Vec<InflightCancel>,
    pub inflight_orders: Vec<ManagedOrder>,
}
