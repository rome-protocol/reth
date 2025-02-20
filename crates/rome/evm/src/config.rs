use alloc::sync::Arc;
use reth_chainspec::ChainSpec;
use rome_sdk::{RomeConfig, rome_evm_client::resources::Payer};

pub struct RomeEvmConfig {
    pub rome: RomeConfig,
    pub chain_spec: Arc<ChainSpec>,
    pub payers: Vec<Payer>
}

