#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "server")]
pub mod server;

mod protocol;
mod serde_impl;
mod util;

pub const PLUGIN_VERSION: &'static str = env!("CARGO_PKG_VERSION");
