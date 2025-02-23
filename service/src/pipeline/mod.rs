//! specialized struct and enum for handling branching of parent-child/first-second related two types.

pub(crate) mod marker;

mod r#enum;
mod r#struct;

pub use r#enum::Pipeline as PipelineE;
pub use r#struct::Pipeline as PipelineT;
