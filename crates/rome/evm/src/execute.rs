//! Ethereum block execution strategy.

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use alloy_consensus::{BlockHeader, Transaction};
use alloy_eips::{eip7685::Requests};
use reth_chainspec::{ChainSpec, EthereumHardforks};
use reth_consensus::ConsensusError;
use reth_ethereum_consensus::validate_block_post_execution;
use reth_evm::{
    execute::{
        BasicBlockExecutorProvider, BlockExecutionError,
        BlockExecutionStrategy, BlockExecutionStrategyFactory, BlockValidationError, ExecuteOutput,
    },
    system_calls::{OnStateHook, SystemCaller},
    ConfigureEvm, Database, Evm,
};
use reth_evm_ethereum::EthEvmConfig;
use reth_chainspec::EthChainSpec;
use reth_primitives::{EthPrimitives, Receipt, RecoveredBlock};
use reth_primitives_traits::{BlockBody, SignedTransaction};
use revm::db::State;
use revm_primitives::{db::DatabaseCommit, ResultAndState};
use rome_sdk::{rome_evm_client::Payer, rome_solana::types::SyncAtomicRpcClient, RomeConfig};
use rome_sdk::rome_evm_client::tx::TxBuilder;
use crate::RomeEvmConfig;

/// Factory for [`RomeExecutionStrategy`].
#[derive(Clone)]
pub struct RomeExecutionStrategyFactory<EvmConfig = EthEvmConfig> {
    /// The chainspec
    chain_spec: Arc<ChainSpec>,
    /// How to create an EVM.
    evm_config: EvmConfig,
    /// Transaction builder
    tx_builder: TxBuilder,
}

impl RomeExecutionStrategyFactory {
    /// Creates a new default ethereum executor strategy factory.
    pub async fn ethereum(
        chain_spec: Arc<ChainSpec>,
        evm_config: EthEvmConfig,
        rome_evm_config: RomeEvmConfig,
    ) -> Self {
        Self::new(
            chain_spec.clone(),
            evm_config,
            rome_evm_config,
        ).await
    }
}

impl<EvmConfig> RomeExecutionStrategyFactory<EvmConfig> {
    /// Creates a new executor strategy factory.
    pub async fn new(
        chain_spec: Arc<ChainSpec>,
        evm_config: EvmConfig,
        rome_evm_config: RomeEvmConfig,
    ) -> Self {
        let payers = Payer::from_config_list(&rome_evm_config.payers).await.unwrap();
        let sync_rpc_client: SyncAtomicRpcClient = Arc::new(rome_evm_config.rome_config.solana_config.clone().into());
        let tx_builder = TxBuilder::new(chain_spec.chain_id(), rome_evm_config.rome_evm_pubkey, sync_rpc_client, payers);

        Self { chain_spec, evm_config, tx_builder }
    }
}

impl<EvmConfig> BlockExecutionStrategyFactory for RomeExecutionStrategyFactory<EvmConfig>
where
    EvmConfig: Clone
    + Unpin
    + Sync
    + Send
    + 'static
    + ConfigureEvm<
        Header = alloy_consensus::Header,
        Transaction = reth_primitives::TransactionSigned,
    >,
{
    type Primitives = EthPrimitives;

    type Strategy<DB: Database> = RomeExecutionStrategy<DB, EvmConfig>;

    fn create_strategy<DB>(&self, db: DB) -> Self::Strategy<DB>
    where
        DB: Database,
    {
        let state =
            State::builder().with_database(db).with_bundle_update().without_state_clear().build();
        RomeExecutionStrategy::new(state, self.chain_spec.clone(), self.evm_config.clone())
    }
}

/// Block execution strategy for Ethereum.
#[allow(missing_debug_implementations)]
pub struct RomeExecutionStrategy<DB, EvmConfig>
where
    EvmConfig: Clone,
{
    /// The chainspec
    chain_spec: Arc<ChainSpec>,
    /// How to create an EVM.
    evm_config: EvmConfig,
    /// Current state for block execution.
    state: State<DB>,
    /// Utility to call system smart contracts.
    system_caller: SystemCaller<EvmConfig, ChainSpec>,
}

impl<DB, EvmConfig> RomeExecutionStrategy<DB, EvmConfig>
where
    EvmConfig: Clone,
{
    /// Creates a new [`RomeExecutionStrategy`]
    pub fn new(state: State<DB>, chain_spec: Arc<ChainSpec>, evm_config: EvmConfig) -> Self {
        let system_caller = SystemCaller::new(evm_config.clone(), chain_spec.clone());
        Self { state, chain_spec, evm_config, system_caller }
    }
}

impl<DB, EvmConfig> BlockExecutionStrategy for RomeExecutionStrategy<DB, EvmConfig>
where
    DB: Database,
    EvmConfig: ConfigureEvm<
        Header = alloy_consensus::Header,
        Transaction = reth_primitives::TransactionSigned,
    >,
{
    type DB = DB;
    type Error = BlockExecutionError;
    type Primitives = EthPrimitives;

    fn apply_pre_execution_changes(
        &mut self,
        block: &RecoveredBlock<reth_primitives::Block>,
    ) -> Result<(), Self::Error> {
        // Set state clear flag if the block is after the Spurious Dragon hardfork.
        let state_clear_flag = self.chain_spec.is_spurious_dragon_active_at_block(block.number());
        self.state.set_state_clear_flag(state_clear_flag);

        let mut evm = self.evm_config.evm_for_block(&mut self.state, block.header());

        self.system_caller.apply_pre_execution_changes(block.header(), &mut evm)?;

        Ok(())
    }

    fn execute_transactions(
        &mut self,
        block: &RecoveredBlock<reth_primitives::Block>,
    ) -> Result<ExecuteOutput<Receipt>, Self::Error> {
        let mut evm = self.evm_config.evm_for_block(&mut self.state, block.header());

        let mut cumulative_gas_used = 0;
        let mut receipts = Vec::with_capacity(block.body().transaction_count());
        for (sender, transaction) in block.transactions_with_sender() {
            // The sum of the transaction's gas limit, Tg, and the gas utilized in this block prior,
            // must be no greater than the block's gasLimit.
            let block_available_gas = block.gas_limit() - cumulative_gas_used;
            if transaction.gas_limit() > block_available_gas {
                return Err(BlockValidationError::TransactionGasLimitMoreThanAvailableBlockGas {
                    transaction_gas_limit: transaction.gas_limit(),
                    block_available_gas,
                }
                    .into())
            }

            let tx_env = self.evm_config.tx_env(transaction, *sender);

            // Execute transaction.
            let result_and_state = evm.transact(tx_env).map_err(move |err| {
                // Ensure hash is calculated for error log, if not already done
                BlockValidationError::EVM {
                    hash: transaction.recalculate_hash(),
                    error: Box::new(err),
                }
            })?;
            self.system_caller.on_state(&result_and_state.state);
            let ResultAndState { result, state } = result_and_state;
            evm.db_mut().commit(state);

            // append gas used
            cumulative_gas_used += result.gas_used();

            // Push transaction changeset and calculate header bloom filter for receipt.
            receipts.push(Receipt {
                tx_type: transaction.tx_type(),
                // Success flag was added in `EIP-658: Embedding transaction status code in
                // receipts`.
                success: result.is_success(),
                cumulative_gas_used,
                logs: result.into_logs(),
            });
        }
        Ok(ExecuteOutput { receipts, gas_used: cumulative_gas_used })
    }

    fn apply_post_execution_changes(
        &mut self,
        block: &RecoveredBlock<reth_primitives::Block>,
        receipts: &[Receipt],
    ) -> Result<Requests, Self::Error> {
        panic!("RomeExecutionStrategy::apply_post_execution_changes")
    }

    fn state_ref(&self) -> &State<DB> {
        &self.state
    }

    fn state_mut(&mut self) -> &mut State<DB> {
        &mut self.state
    }

    fn into_state(self) -> State<Self::DB> {
        self.state
    }

    fn with_state_hook(&mut self, hook: Option<Box<dyn OnStateHook>>) {
        self.system_caller.with_state_hook(hook);
    }

    fn validate_block_post_execution(
        &self,
        block: &RecoveredBlock<reth_primitives::Block>,
        receipts: &[Receipt],
        requests: &Requests,
    ) -> Result<(), ConsensusError> {
        validate_block_post_execution(block, &self.chain_spec.clone(), receipts, requests)
    }
}

/// Helper type with backwards compatible methods to obtain Ethereum executor
/// providers.
#[derive(Debug)]
pub struct RomeExecutorProvider;

impl RomeExecutorProvider {
    /// Creates a new default ethereum executor provider.
    pub async fn ethereum(
        chain_spec: Arc<ChainSpec>,
        evm_config: EthEvmConfig,
        rome_evm_config: RomeEvmConfig,
    ) -> BasicBlockExecutorProvider<RomeExecutionStrategyFactory> {
        BasicBlockExecutorProvider::new(
            RomeExecutionStrategyFactory::ethereum(
                chain_spec,
                evm_config,
                rome_evm_config,
            ).await,
        )
    }
}
