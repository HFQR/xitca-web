//! A middleware construct self from `Option<T: Service<S: Service<Req>>>`.

use core::future::Future;

use crate::pipeline::PipelineE;

use super::Service;

impl<T, Arg> Service<Arg> for Option<T>
where
    T: Service<Arg>,
{
    type Response = PipelineE<Arg, T::Response>;
    type Error = T::Error;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

    fn call(&self, arg: Arg) -> Self::Future<'_> {
        async move {
            match self {
                None => Ok(PipelineE::First(arg)),
                Some(ref t) => {
                    let res = t.call(arg).await?;
                    Ok(PipelineE::Second(res))
                }
            }
        }
    }
}
