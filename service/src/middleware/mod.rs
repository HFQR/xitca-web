//! utility middlewares that can be used together with [ServiceExt::enclosed].
//!
//! [ServiceExt::enclosed]: crate::service::ServiceExt::enclosed

mod async_fn;
mod group;
mod map;
mod unchecked_ready;

pub use async_fn::AsyncFn;
pub use group::Group;
pub use unchecked_ready::UncheckedReady;

pub(crate) use map::{Map, MapErr};

use crate::pipeline::{PipelineT, marker::BuildEnclosed};

/// Type alias for specialized [PipelineT] type.
pub type EnclosedBuilder<F, S> = PipelineT<F, S, BuildEnclosed>;

/// Type alias for specialized [PipelineT] type.
pub type EnclosedFnBuilder<F, S> = EnclosedBuilder<F, AsyncFn<S>>;

/// Type alias for specialized [PipelineT] type.
pub type MapBuilder<F, S> = EnclosedBuilder<F, Map<S>>;

/// Type alias for specialized [PipelineT] type.
pub type MapErrorBuilder<F, S> = EnclosedBuilder<F, MapErr<S>>;
