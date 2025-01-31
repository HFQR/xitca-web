use core::{
    convert::Infallible,
    error::Error,
    fmt::{self, Debug, Display, Formatter},
};

use crate::{ready::ReadyService, service::Service};

/// A pipeline type where two variants have a either/or relationship
pub enum Pipeline<F, S> {
    First(F),
    Second(S),
}

impl<F, S> Clone for Pipeline<F, S>
where
    F: Clone,
    S: Clone,
{
    fn clone(&self) -> Self {
        match *self {
            Self::First(ref p) => Self::First(p.clone()),
            Self::Second(ref p) => Self::Second(p.clone()),
        }
    }
}

impl<F, S> Pipeline<F, S>
where
    F: From<S>,
{
    pub fn into_first(self) -> F {
        match self {
            Self::First(f) => f,
            Self::Second(s) => F::from(s),
        }
    }
}

impl<F, S> Pipeline<F, S>
where
    S: From<F>,
{
    pub fn into_second(self) -> S {
        match self {
            Self::First(f) => S::from(f),
            Self::Second(s) => s,
        }
    }
}

impl<F, S> Debug for Pipeline<F, S>
where
    F: Debug,
    S: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::First(ref p) => Debug::fmt(p, f),
            Self::Second(ref p) => Debug::fmt(p, f),
        }
    }
}

impl<F, S> Display for Pipeline<F, S>
where
    F: Display,
    S: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::First(ref p) => Display::fmt(p, f),
            Self::Second(ref p) => Display::fmt(p, f),
        }
    }
}

// useful default impl when pipeline enum used as error type.
impl<F, S> From<Infallible> for Pipeline<F, S> {
    fn from(e: Infallible) -> Self {
        match e {}
    }
}

impl<F, S, Req> Service<Req> for Pipeline<F, S>
where
    F: Service<Req>,
    S: Service<Req, Response = F::Response, Error = F::Error>,
{
    type Response = F::Response;
    type Error = F::Error;

    async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
        match *self {
            Self::First(ref f) => f.call(req).await,
            Self::Second(ref s) => s.call(req).await,
        }
    }
}

impl<F, S> ReadyService for Pipeline<F, S>
where
    F: ReadyService,
    S: ReadyService,
{
    type Ready = Pipeline<F::Ready, S::Ready>;

    #[inline]
    async fn ready(&self) -> Self::Ready {
        match *self {
            Self::First(ref f) => Pipeline::First(f.ready().await),
            Self::Second(ref s) => Pipeline::Second(s.ready().await),
        }
    }
}

impl<F, S> Error for Pipeline<F, S>
where
    F: Error,
    S: Error,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match *self {
            Self::First(ref f) => Error::source(f),
            Self::Second(ref f) => Error::source(f),
        }
    }
}
