mod extension;
mod logger;

#[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
mod tcp_config;

pub use extension::Extension;
pub use logger::Logger;

#[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
pub use tcp_config::TcpConfig;
