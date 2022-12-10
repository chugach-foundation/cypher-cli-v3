use thiserror::Error;

pub mod hedging;
pub mod making;

#[derive(Debug, Error)]
pub enum StrategyError {}
