//! Rows. mostly copy/paste from `tokio-postgres`

mod traits;
mod types;

#[cfg(feature = "compat")]
pub(crate) mod compat;

pub use types::{Row, RowSimple};
