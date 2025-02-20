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
use reth_rome_evm::RomeEvmConfig;
use std::{fs::File, path::PathBuf};
use std::io::Read;
use serde_yaml;

fn main() {
    reth_cli_util::sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    if std::env::var_os("ROME_EVM_CONFIG").is_none() {
        panic!("ROME_EVM_CONFIG env variable must be set");
    }

    let rome_evm_config_path =  std::env::var("ROME_EVM_CONFIG")
        .ok()
        .map(PathBuf::from)
        .unwrap();

    let mut rome_evm_config_buf = Vec::new();
    File::open(rome_evm_config_path.clone())
        .unwrap()
        .read_to_end(&mut rome_evm_config_buf)
        .unwrap();

    // Get the extension of the file
    let ext = rome_evm_config_path.extension().map(|ext| ext.to_str().unwrap_or_default());

    // Parse according to the extension
    let rome_evm_config = match ext {
        Some("yml") | Some("yaml") => {
            let value =
                serde_yaml::from_slice(&rome_evm_config_buf).expect("Failed to parse config file as YAML");

            serde_yaml::from_value::<RomeEvmConfig>(value).expect("Error parsing config file")
        }
        _ => {
            let value =
                serde_json::from_slice(&rome_evm_config_buf).expect("Failed to parse config file as JSON");

            serde_json::from_value::<RomeEvmConfig>(value).expect("Error parsing config file")
        }
    };

    if let Err(err) = Cli::<EthereumChainSpecParser>::parse().run(|builder, _| async move {
        info!(target: "reth::cli", "Launching node");
        let handle = builder.launch_node(RomeNode::new(rome_evm_config)).await?;
        handle.node_exit_future.await
    }) {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
