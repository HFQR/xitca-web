//! specialized struct and enum for handling branching of parent-child/first-second related two types.

pub(crate) mod marker;

mod r#enum;
mod r#struct;

pub use r#enum::Pipeline as PipelineE;
pub use r#struct::Pipeline as PipelineT;

/// Type alias for specialized [PipelineT] type.
pub type EnclosedFnBuilder<F, S> = PipelineT<F, crate::middleware::AsyncFn<S>, marker::BuildEnclosed>;

/// Type alias for specialized [PipelineT] type.
pub type EnclosedBuilder<F, S> = PipelineT<F, S, marker::BuildEnclosed>;

/// Type alias for specialized [PipelineT] type.
pub type MapBuilder<F, S> = PipelineT<F, S, marker::BuildMap>;

/// Type alias for specialized [PipelineT] type.
pub type MapErrorBuilder<F, S> = PipelineT<F, S, marker::BuildMapErr>;
