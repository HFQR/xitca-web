//! Marker types for different variant of [crate::pipeline::PipelineT]

use core::marker::PhantomData;

pub struct Map;
pub struct MapErr;
pub struct AndThen;
pub struct Enclosed;
pub struct EnclosedFn<Req>(PhantomData<Req>);
