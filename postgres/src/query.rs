mod base;
mod row_stream;
mod simple;

#[cfg(feature = "compat")]
pub(crate) mod compat;

pub use base::{Query, RowStream};
pub use simple::{QuerySimple, RowSimpleStream};

use core::{
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

use super::{driver::codec::Response, error::Error};

pub struct ExecuteFuture {
    res: Result<Response, Option<Error>>,
    rows_affected: u64,
}

impl Future for ExecuteFuture {
    type Output = Result<u64, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        match this.res {
            Ok(ref mut res) => {
                ready!(res.poll_try_into_ready(&mut this.rows_affected, cx))?;
                Poll::Ready(Ok(this.rows_affected))
            }
            Err(ref mut e) => Poll::Ready(Err(e.take().expect("ExecuteFuture polled after finish"))),
        }
    }
}
