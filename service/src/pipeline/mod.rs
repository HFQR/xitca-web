pub(crate) mod marker;

mod pipeline_enum;
mod pipeline_struct;

pub use pipeline_enum::Pipeline as PipelineE;
pub use pipeline_struct::Pipeline as PipelineT;

/// Type alias for specialized [PipelineT] type with [marker::EnclosedFn].
pub type EnclosedFnFactory<F, S, Req> = PipelineT<F, S, marker::EnclosedFn<Req>>;

/// Type alias for specialized [PipelineT] type with [marker::Enclosed].
pub type EnclosedFactory<F, S> = PipelineT<F, S, marker::Enclosed>;

/// Type alias for specialized [PipelineT] type with [marker::MapErr].
pub type MapErrorServiceFactory<F, S> = PipelineT<F, S, marker::MapErr>;
