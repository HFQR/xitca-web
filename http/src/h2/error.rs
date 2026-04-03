use crate::error::HttpServiceError;

#[derive(Debug)]
pub enum Error<S, B> {
    Service(S),
    Body(B),
}

impl<S, B> From<Error<S, B>> for HttpServiceError<S, B> {
    fn from(e: Error<S, B>) -> Self {
        match e {
            Error::Service(e) => Self::Service(e),
            e => Self::H2(e),
        }
    }
}
