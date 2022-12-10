use async_trait::async_trait;
use cypher_utils::contexts::Order;
use tokio::sync::RwLock;

use crate::common::orders::{CandidateCancel, CandidatePlacement, OrderManager, OrderManagerError};

pub struct PerpsOrderManager {
    pub open_orders: RwLock<Vec<Order>>,
    pub inflight_orders: RwLock<Vec<Order>>,
}

impl PerpsOrderManager {
    pub fn new() -> Self {
        Self {
            open_orders: RwLock::new(Vec::new()),
            inflight_orders: RwLock::new(Vec::new()),
        }
    }
}

#[async_trait]
impl OrderManager for PerpsOrderManager {
    async fn update(&self) -> Result<(), OrderManagerError> {
        // get orders in the open orders account
        // compare those to inflight_orders client ids
        // if they match, remove corresponding order from inflight_orders and add to open_orders

        Ok(())
    }

    async fn get_candidate_cancels(&self) -> Result<Vec<CandidateCancel>, OrderManagerError> {
        // receive target order sizes and prices
        Ok(Vec::new())
    }

    async fn get_candidate_placements(&self) -> Result<Vec<CandidatePlacement>, OrderManagerError> {
        Ok(Vec::new())
    }
}
