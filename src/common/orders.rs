use async_trait::async_trait;
use cypher_client::Side;
use thiserror::Error;

#[derive(Default, Debug)]
pub struct CandidateCancel {
    /// The order's id.
    pub order_id: u128,
    /// The order's client id.
    pub client_order_id: u64,
    /// The order's side.
    pub side: Side,
}

#[derive(Default, Debug)]
pub struct CandidatePlacement {
    /// The order's price.
    pub price: u64,
    /// The order's base quantity.
    pub base_quantity: u64,
    /// The order's max quote quantity.
    pub max_quote_quantity: u64,
    /// The order's time in force timestamp.
    pub max_ts: u64,
    /// The order's client id.
    pub client_order_id: u64,
    /// The order's side.
    pub side: Side,
}

#[derive(Debug, Error)]
pub enum OrderManagerError {}

/// Defines shared functionality that different order managers should implement.
#[async_trait]
pub trait OrderManager: Send + Sync {
    /// Triggers an update of the order manager, which should process incoming data and
    /// cache existing open orders and update inflight orders.
    async fn update(&self) -> Result<(), OrderManagerError>;

    /// Gets candidate order cancels according to the given data.
    ///
    /// This should be called after calculating optimal quote sizes and prices, so cancels happen
    /// optimally, if they need to happen at all.
    async fn get_candidate_cancels(&self) -> Result<Vec<CandidateCancel>, OrderManagerError>;

    /// Gets candidate order placements according to the given data.
    ///
    /// This should be called after calculating the optimal quote sizes and prices, so placements
    /// happen optimally, if they need to happen at all.
    async fn get_candidate_placements(&self) -> Result<Vec<CandidatePlacement>, OrderManagerError>;
}
