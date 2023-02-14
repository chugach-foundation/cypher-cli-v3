use serde::{Deserialize, Serialize};




#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiquidatorConfig {
    pub rebalance: bool,
}
