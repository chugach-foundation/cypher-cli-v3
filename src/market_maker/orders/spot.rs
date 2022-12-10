use async_trait::async_trait;
use cypher_utils::contexts::Order;
use tokio::sync::RwLock;

use crate::common::orders::{CandidateCancel, CandidatePlacement, OrderManager, OrderManagerError};

pub struct SpotOrderManager {
    pub open_orders: RwLock<Vec<Order>>,
    pub inflight_orders: RwLock<Vec<Order>>,
}

impl SpotOrderManager {
    pub fn new() -> Self {
        Self {
            open_orders: RwLock::new(Vec::new()),
            inflight_orders: RwLock::new(Vec::new()),
        }
    }
}

#[async_trait]
impl OrderManager for SpotOrderManager {
    async fn update(&self) -> Result<(), OrderManagerError> {
        Ok(())
    }

    async fn get_candidate_cancels(&self) -> Result<Vec<CandidateCancel>, OrderManagerError> {
        Ok(Vec::new())
    }

    async fn get_candidate_placements(&self) -> Result<Vec<CandidatePlacement>, OrderManagerError> {
        Ok(Vec::new())
    }
}
