mod stream;

#[cfg(feature = "compat")]
pub(crate) mod compat;

pub use stream::{RowAffected, RowSimpleStream, RowSimpleStreamOwned, RowStream, RowStreamGuarded, RowStreamOwned};
