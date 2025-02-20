#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

extern crate alloc;

pub mod execute;
pub mod config;

pub use config::RomeEvmConfig;
pub use execute::RomeExecutionStrategyFactory;
