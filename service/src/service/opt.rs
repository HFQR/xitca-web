//! A middleware construct self from `Option<T: Service<S: Service<Req>>>`.

use crate::pipeline::PipelineE;

use super::Service;

impl<T, S, E> Service<Result<S, E>> for Option<T>
where
    T: Service<Result<S, E>, Error = E>,
{
    type Response = PipelineE<S, T::Response>;
    type Error = T::Error;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        match *self {
            None => res.map(PipelineE::First),
            Some(ref t) => t.call(res).await.map(PipelineE::Second),
        }
    }
}
