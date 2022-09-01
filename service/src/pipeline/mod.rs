//! specialized struct and enum for handling branching of parent-child/first-second related two types.

pub(crate) mod marker;

mod r#enum;
mod r#struct;

pub use r#enum::Pipeline as PipelineE;
pub use r#struct::Pipeline as PipelineT;

/// Type alias for specialized [PipelineT] type with [marker::EnclosedFn].
pub type EnclosedFnFactory<F, S> = PipelineT<F, S, marker::EnclosedFn>;

/// Type alias for specialized [PipelineT] type with [marker::Enclosed].
pub type EnclosedFactory<F, S> = PipelineT<F, S, marker::Enclosed>;

/// Type alias for specialized [PipelineT] type with [marker::MapErr].
pub type MapErrorServiceFactory<F, S> = PipelineT<F, S, marker::MapErr>;
