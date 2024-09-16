//! compatible types for Row that can work with futures::Stream trait

use core::ops::Range;

use std::sync::Arc;

use crate::column::Column;

use super::types::{marker, GenericRow};

/// A row of data returned from the database by a query.
pub type RowOwned = GenericRow<Arc<[Column]>, Vec<Range<usize>>, marker::Typed>;
