//! Rome Node types config.
use reth_chainspec::ChainSpec;
pub use reth_ethereum_engine_primitives::{EthereumEngineValidator, EthEngineTypes };
use reth_ethereum_engine_primitives::{
    EthBuiltPayload, EthPayloadAttributes, EthPayloadBuilderAttributes,
};
use reth_ethereum_primitives::EthPrimitives;
use reth_evm::{execute::BasicBlockExecutorProvider, ConfigureEvm};
use reth_evm_ethereum::{EthEvmConfig};
use reth_node_api::{AddOnsContext, FullNodeComponents, NodeAddOns};
use reth_node_builder::{
    components::{
        ComponentsBuilder, ExecutorBuilder,
    },
    node::{FullNodeTypes, NodeTypes, NodeTypesWithEngine},
    rpc::{EngineValidatorAddOn, EngineValidatorBuilder, RethRpcAddOns, RpcAddOns, RpcHandle},
    BuilderContext, Node, NodeAdapter, NodeComponentsBuilder, PayloadTypes,
};
use reth_node_ethereum::node::{EthereumConsensusBuilder, EthereumPoolBuilder, EthereumEngineValidatorBuilder, EthereumNetworkBuilder, EthereumPayloadBuilder};
use reth_provider::{providers::ProviderFactoryBuilder, EthStorage};
use reth_rome_evm::RomeExecutionStrategyFactory;
use reth_rpc::{eth::core::EthApiFor, ValidationApi};
use reth_rpc_api::servers::BlockSubmissionValidationApiServer;
use reth_rpc_builder::config::RethRpcServerConfig;
use reth_rpc_eth_types::{error::FromEvmError, EthApiError};
use reth_rpc_server_types::RethRpcModule;

use reth_trie_db::MerklePatriciaTrie;
use revm::primitives::TxEnv;
use std::sync::Arc;
use reth_rome_evm::config::RomeEvmConfig;

/// Type configuration for a regular Rome node.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RomeNode {
    pub rome_evm_config: RomeEvmConfig,
}

impl RomeNode {
    /// Configure the data availability configuration for the OP builder.
    pub fn new(rome_evm_config: RomeEvmConfig) -> Self {
        RomeNode { rome_evm_config }
    }

    /// Returns a [`ComponentsBuilder`] configured for a regular Rome node.
    pub fn components<Node>(&self) -> ComponentsBuilder<
        Node,
        EthereumPoolBuilder,
        EthereumPayloadBuilder,
        EthereumNetworkBuilder,
        RomeExecutorBuilder,
        EthereumConsensusBuilder,
    >
    where
        Node: FullNodeTypes<Types: NodeTypes<ChainSpec = ChainSpec, Primitives = EthPrimitives>>,
        <Node::Types as NodeTypesWithEngine>::Engine: PayloadTypes<
            BuiltPayload = EthBuiltPayload,
            PayloadAttributes = EthPayloadAttributes,
            PayloadBuilderAttributes = EthPayloadBuilderAttributes,
        >,
    {
        ComponentsBuilder::default()
            .node_types::<Node>()
            .pool(EthereumPoolBuilder::default())
            .payload(EthereumPayloadBuilder::default())
            .network(EthereumNetworkBuilder::default())
            .executor(RomeExecutorBuilder::new(self.rome_evm_config.clone()))
            .consensus(EthereumConsensusBuilder::default())
    }

    /// Instantiates the [`ProviderFactoryBuilder`] for an ethereum node.
    ///
    /// # Open a Providerfactory in read-only mode from a datadir
    ///
    /// See also: [`ProviderFactoryBuilder`] and
    /// [`ReadOnlyConfig`](reth_provider::providers::ReadOnlyConfig).
    ///
    /// ```no_run
    /// use reth_chainspec::MAINNET;
    /// use reth_rome_node::RomeNode;
    ///
    /// let factory = RomeNode::provider_factory_builder()
    ///     .open_read_only(MAINNET.clone(), "datadir")
    ///     .unwrap();
    /// ```
    ///
    /// # Open a Providerfactory manually with with all required components
    ///
    /// ```no_run
    /// use reth_chainspec::ChainSpecBuilder;
    /// use reth_db::open_db_read_only;
    /// use reth_rome_node::RomeNode;
    /// use reth_provider::providers::StaticFileProvider;
    /// use std::sync::Arc;
    ///
    /// let factory = RomeNode::provider_factory_builder()
    ///     .db(Arc::new(open_db_read_only("db", Default::default()).unwrap()))
    ///     .chainspec(ChainSpecBuilder::mainnet().build().into())
    ///     .static_file(StaticFileProvider::read_only("db/static_files", false).unwrap())
    ///     .build_provider_factory();
    /// ```
    pub fn provider_factory_builder() -> ProviderFactoryBuilder<Self> {
        ProviderFactoryBuilder::default()
    }
}

impl NodeTypes for RomeNode {
    type Primitives = EthPrimitives;
    type ChainSpec = ChainSpec;
    type StateCommitment = MerklePatriciaTrie;
    type Storage = EthStorage;
}

impl NodeTypesWithEngine for RomeNode {
    type Engine = EthEngineTypes;
}

/// Add-ons w.r.t. l1 ethereum.
#[derive(Debug)]
pub struct RomeAddOns<N: FullNodeComponents> {
    inner: RpcAddOns<N, EthApiFor<N>, EthereumEngineValidatorBuilder>,
}

impl<N: FullNodeComponents> Default for RomeAddOns<N> {
    fn default() -> Self {
        Self { inner: Default::default() }
    }
}

impl<N> NodeAddOns<N> for RomeAddOns<N>
where
    N: FullNodeComponents<
        Types: NodeTypesWithEngine<
            ChainSpec = ChainSpec,
            Primitives = EthPrimitives,
            Engine = EthEngineTypes,
        >,
        Evm: ConfigureEvm<TxEnv = TxEnv>,
    >,
    EthApiError: FromEvmError<N::Evm>,
{
    type Handle = RpcHandle<N, EthApiFor<N>>;

    async fn launch_add_ons(
        self,
        ctx: reth_node_api::AddOnsContext<'_, N>,
    ) -> eyre::Result<Self::Handle> {
        let validation_api = ValidationApi::new(
            ctx.node.provider().clone(),
            Arc::new(ctx.node.consensus().clone()),
            ctx.node.block_executor().clone(),
            ctx.config.rpc.flashbots_config(),
            Box::new(ctx.node.task_executor().clone()),
            Arc::new(EthereumEngineValidator::new(ctx.config.chain.clone())),
        );

        self.inner
            .launch_add_ons_with(ctx, move |modules, _| {
                modules.merge_if_module_configured(
                    RethRpcModule::Flashbots,
                    validation_api.into_rpc(),
                )?;

                Ok(())
            })
            .await
    }
}

impl<N> RethRpcAddOns<N> for RomeAddOns<N>
where
    N: FullNodeComponents<
        Types: NodeTypesWithEngine<
            ChainSpec = ChainSpec,
            Primitives = EthPrimitives,
            Engine = EthEngineTypes,
        >,
        Evm: ConfigureEvm<TxEnv = TxEnv>,
    >,
    EthApiError: FromEvmError<N::Evm>,
{
    type EthApi = EthApiFor<N>;

    fn hooks_mut(&mut self) -> &mut reth_node_builder::rpc::RpcHooks<N, Self::EthApi> {
        self.inner.hooks_mut()
    }
}

impl<N> EngineValidatorAddOn<N> for RomeAddOns<N>
where
    N: FullNodeComponents<
        Types: NodeTypesWithEngine<
            ChainSpec = ChainSpec,
            Primitives = EthPrimitives,
            Engine = EthEngineTypes,
        >,
    >,
{
    type Validator = EthereumEngineValidator;

    async fn engine_validator(&self, ctx: &AddOnsContext<'_, N>) -> eyre::Result<Self::Validator> {
        EthereumEngineValidatorBuilder::default().build(ctx).await
    }
}

impl<N> Node<N> for RomeNode
where
    N: FullNodeTypes<Types = Self>,
{
    type ComponentsBuilder = ComponentsBuilder<
        N,
        EthereumPoolBuilder,
        EthereumPayloadBuilder,
        EthereumNetworkBuilder,
        RomeExecutorBuilder,
        EthereumConsensusBuilder,
    >;

    type AddOns = RomeAddOns<
        NodeAdapter<N, <Self::ComponentsBuilder as NodeComponentsBuilder<N>>::Components>,
    >;

    fn components_builder(&self) -> Self::ComponentsBuilder {
        Self::components(self)
    }

    fn add_ons(&self) -> Self::AddOns {
        RomeAddOns::default()
    }
}

/// A regular ethereum evm and executor builder.
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct RomeExecutorBuilder {
    rome_evm_config: RomeEvmConfig
}

impl RomeExecutorBuilder {
    pub fn new(rome_evm_config: RomeEvmConfig) -> Self {
        Self { rome_evm_config }
    }
}

impl<Types, Node> ExecutorBuilder<Node> for RomeExecutorBuilder
where
    Types: NodeTypesWithEngine<ChainSpec = ChainSpec, Primitives = EthPrimitives>,
    Node: FullNodeTypes<Types = Types>,
{
    type EVM = EthEvmConfig;
    type Executor = BasicBlockExecutorProvider<RomeExecutionStrategyFactory>;

    async fn build_evm(
        self,
        ctx: &BuilderContext<Node>,
    ) -> eyre::Result<(Self::EVM, Self::Executor)> {
        let chain_spec = ctx.chain_spec();
        let evm_config = EthEvmConfig::new(ctx.chain_spec());
        let strategy_factory = RomeExecutionStrategyFactory::new(chain_spec, evm_config.clone(), self.rome_evm_config).await; // todo;
        let executor = BasicBlockExecutorProvider::new(strategy_factory);

        Ok((evm_config, executor))
    }
}
