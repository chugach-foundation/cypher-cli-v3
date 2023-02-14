use serde::{Deserialize, Serialize};
use serde_json;

use crate::config::PersistentConfig;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiquidatorConfig {
    pub rebalance: bool,
}
