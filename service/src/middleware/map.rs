use crate::{
    pipeline::{PipelineT, marker},
    service::Service,
};

pub struct Map<F>(pub F);

impl<F> Clone for Map<F>
where
    F: Clone,
{
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S, E, F> Service<Result<S, E>> for Map<F>
where
    F: Clone,
{
    type Response = PipelineT<S, F, marker::Map>;
    type Error = E;

    async fn call(&self, arg: Result<S, E>) -> Result<Self::Response, Self::Error> {
        arg.map(|service| PipelineT::new(service, self.0.clone()))
    }
}

impl<S, Req, F, Res> Service<Req> for PipelineT<S, F, marker::Map>
where
    S: Service<Req>,
    F: Fn(S::Response) -> Res,
{
    type Response = Res;
    type Error = S::Error;

    #[inline]
    async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
        self.first.call(req).await.map(&self.second)
    }
}

#[derive(Clone)]
pub struct MapErr<F>(pub F);

impl<S, E, F> Service<Result<S, E>> for MapErr<F>
where
    F: Clone,
{
    type Response = PipelineT<S, F, marker::MapErr>;
    type Error = E;

    async fn call(&self, arg: Result<S, E>) -> Result<Self::Response, Self::Error> {
        arg.map(|service| PipelineT::new(service, self.0.clone()))
    }
}

impl<S, Req, F, Err> Service<Req> for PipelineT<S, F, marker::MapErr>
where
    S: Service<Req>,
    F: Fn(S::Error) -> Err,
{
    type Response = S::Response;
    type Error = Err;

    #[inline]
    async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
        self.first.call(req).await.map_err(&self.second)
    }
}
