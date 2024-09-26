//! Rows. mostly copy/paste from `tokio-postgres`

mod traits;
mod types;

pub use types::{Row, RowOwned, RowSimple, RowSimpleOwned};

// Marker types for specialized impl on row types
pub(crate) mod marker {
    #[derive(Debug)]
    pub struct Typed;
    #[derive(Debug)]

    pub struct NoTyped;
}
