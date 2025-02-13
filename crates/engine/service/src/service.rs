use futures::{Stream, StreamExt};
use pin_project::pin_project;
use reth_chainspec::EthChainSpec;
use reth_consensus::{ConsensusError, FullConsensus};
use reth_engine_primitives::{BeaconConsensusEngineEvent, BeaconEngineMessage, EngineValidator};
use reth_engine_tree::{
    backfill::PipelineSync,
    download::BasicBlockDownloader,
    engine::{EngineApiKind, EngineApiRequest, EngineApiRequestHandler, EngineHandler},
    persistence::PersistenceHandle,
    tree::{EngineApiTreeHandler, InvalidBlockHook, TreeConfig},
};
pub use reth_engine_tree::{
    chain::{ChainEvent, ChainOrchestrator},
    engine::EngineApiEvent,
};
use reth_evm::{execute::BlockExecutorProvider, ConfigureEvm};
use reth_network_p2p::BlockClient;
use reth_node_types::{BlockTy, HeaderTy, NodeTypes, NodeTypesWithEngine, TxTy};
use reth_payload_builder::PayloadBuilderHandle;
use reth_primitives::EthPrimitives;
use reth_provider::{
    providers::{BlockchainProvider, EngineNodeTypes},
    ProviderFactory,
};
use reth_prune::PrunerWithFactory;
use reth_stages_api::{MetricEventsSender, Pipeline};
use reth_tasks::TaskSpawner;
use std::{
    marker::PhantomData,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

/// Alias for consensus engine stream.
pub type EngineMessageStream<T> = Pin<Box<dyn Stream<Item = BeaconEngineMessage<T>> + Send + Sync>>;

/// Alias for chain orchestrator.
type EngineServiceType<N, Client> = ChainOrchestrator<
    EngineHandler<
        EngineApiRequestHandler<
            EngineApiRequest<<N as NodeTypesWithEngine>::Engine, <N as NodeTypes>::Primitives>,
            <N as NodeTypes>::Primitives,
        >,
        EngineMessageStream<<N as NodeTypesWithEngine>::Engine>,
        BasicBlockDownloader<Client, BlockTy<N>>,
    >,
    PipelineSync<N>,
>;

/// The type that drives the chain forward and communicates progress.
#[pin_project]
#[allow(missing_debug_implementations)]
// TODO(mattsse): remove hidde once fixed : <https://github.com/rust-lang/rust/issues/135363>
//  otherwise rustdoc fails to resolve the alias
#[doc(hidden)]
pub struct EngineService<N, Client, E>
where
    N: EngineNodeTypes,
    Client: BlockClient<Block = BlockTy<N>> + 'static,
    E: BlockExecutorProvider + 'static,
{
    orchestrator: EngineServiceType<N, Client>,
    _marker: PhantomData<E>,
}

impl<N, Client, E> EngineService<N, Client, E>
where
    N: EngineNodeTypes,
    Client: BlockClient<Block = BlockTy<N>> + 'static,
    E: BlockExecutorProvider<Primitives = N::Primitives> + 'static,
{
    /// Constructor for `EngineService`.
    #[allow(clippy::too_many_arguments)]
    pub fn new<V, C>(
        consensus: Arc<dyn FullConsensus<N::Primitives, Error = ConsensusError>>,
        executor_factory: E,
        chain_spec: Arc<N::ChainSpec>,
        client: Client,
        incoming_requests: EngineMessageStream<N::Engine>,
        pipeline: Pipeline<N>,
        pipeline_task_spawner: Box<dyn TaskSpawner>,
        provider: ProviderFactory<N>,
        blockchain_db: BlockchainProvider<N>,
        pruner: PrunerWithFactory<ProviderFactory<N>>,
        payload_builder: PayloadBuilderHandle<N::Engine>,
        payload_validator: V,
        tree_config: TreeConfig,
        invalid_block_hook: Box<dyn InvalidBlockHook<N::Primitives>>,
        sync_metrics_tx: MetricEventsSender,
        evm_config: C,
    ) -> Self
    where
        V: EngineValidator<N::Engine, Block = BlockTy<N>>,
        C: ConfigureEvm<Header = HeaderTy<N>, Transaction = TxTy<N>>,
    {
        let engine_kind =
            if chain_spec.is_optimism() { EngineApiKind::OpStack } else { EngineApiKind::Ethereum };

        let downloader = BasicBlockDownloader::new(client, consensus.clone().as_consensus());

        let persistence_handle =
            PersistenceHandle::<EthPrimitives>::spawn_service(provider, pruner, sync_metrics_tx);

        let canonical_in_memory_state = blockchain_db.canonical_in_memory_state();

        let (to_tree_tx, from_tree) =
            EngineApiTreeHandler::<N::Primitives, _, _, _, _, _>::spawn_new(
                blockchain_db,
                executor_factory,
                consensus,
                payload_validator,
                persistence_handle,
                payload_builder,
                canonical_in_memory_state,
                tree_config,
                invalid_block_hook,
                engine_kind,
                evm_config,
            );

        let engine_handler = EngineApiRequestHandler::new(to_tree_tx, from_tree);
        let handler = EngineHandler::new(engine_handler, downloader, incoming_requests);

        let backfill_sync = PipelineSync::new(pipeline, pipeline_task_spawner);

        Self {
            orchestrator: ChainOrchestrator::new(handler, backfill_sync),
            _marker: Default::default(),
        }
    }

    /// Returns a mutable reference to the orchestrator.
    pub fn orchestrator_mut(&mut self) -> &mut EngineServiceType<N, Client> {
        &mut self.orchestrator
    }
}

impl<N, Client, E> Stream for EngineService<N, Client, E>
where
    N: EngineNodeTypes,
    Client: BlockClient<Block = BlockTy<N>> + 'static,
    E: BlockExecutorProvider + 'static,
{
    type Item = ChainEvent<BeaconConsensusEngineEvent<N::Primitives>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut orchestrator = self.project().orchestrator;
        StreamExt::poll_next_unpin(&mut orchestrator, cx)
    }
}

/// Potential error returned by `EngineService`.
#[derive(Debug, thiserror::Error)]
#[error("Engine service error.")]
pub struct EngineServiceError {}
