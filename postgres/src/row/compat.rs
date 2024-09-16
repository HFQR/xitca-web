//! compatible types for Row that can work with futures::Stream trait

use core::ops::Range;

use std::sync::Arc;

use crate::column::Column;

use super::types::{marker, GenericRow};

pub type RowOwned = GenericRow<Arc<[Column]>, Vec<Range<usize>>, marker::Typed>;
