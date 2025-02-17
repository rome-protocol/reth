#![allow(missing_docs, rustdoc::missing_crate_level_docs)]
// The `rome` feature must be enabled to use this crate.
#![cfg(feature = "rome")]
#[global_allocator]
static ALLOC: reth_cli_util::allocator::Allocator = reth_cli_util::allocator::new_allocator();

use clap::Parser;
use reth_ethereum_cli::chainspec::EthereumChainSpecParser;
use reth_rome_node::RomeNode;
use tracing::info;
use rome_reth::cli::Cli;

fn main() {
    reth_cli_util::sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    if let Err(err) = Cli::<EthereumChainSpecParser>::parse().run(|builder, _| async move {
        info!(target: "reth::cli", "Launching node");
        let handle = builder.launch_node(RomeNode::default()).await?;
        handle.node_exit_future.await
    }) {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
