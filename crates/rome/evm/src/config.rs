use rome_sdk::{RomeConfig, Pubkey};
use rome_sdk::rome_evm_client::PayerConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RomeEvmConfig {
    pub rome_config: RomeConfig,
    pub rome_evm_pubkey: Pubkey,
    pub payers: Vec<PayerConfig>,
}
