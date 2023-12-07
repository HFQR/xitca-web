//! specialized struct and enum for handling branching of parent-child/first-second related two types.

pub(crate) mod marker;

mod r#enum;
mod r#struct;

pub use r#enum::Pipeline as PipelineE;
pub use r#struct::Pipeline as PipelineT;

/// Type alias for specialized [PipelineT] type.
pub type EnclosedFnFactory<F, S> = PipelineT<F, S, marker::BuildEnclosedFn>;

/// Type alias for specialized [PipelineT] type.
pub type EnclosedFactory<F, S> = PipelineT<F, S, marker::BuildEnclosed>;

/// Type alias for specialized [PipelineT] type.
pub type MapErrorServiceFactory<F, S> = PipelineT<F, S, marker::BuildMapErr>;
