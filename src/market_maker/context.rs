use std::sync::Arc;

use cypher_utils::{
    accounts_cache::AccountsCache,
    contexts::{Order, OrderBook},
};

pub struct Context {
    pub orderbook: OrderBook,
    pub open_orders: Vec<Order>,
}

pub struct ContextBuilder {
    accounts_cache: Arc<AccountsCache>,
}

impl ContextBuilder {
    pub fn new(accounts_cache: Arc<AccountsCache>) -> Self {
        Self { accounts_cache }
    }
}

pub trait MarketMaker {}
