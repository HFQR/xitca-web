use core::future::Future;

use postgres_protocol::message::{backend, frontend};
use xitca_io::bytes::{Buf, Bytes, BytesMut};

use super::{
    client::{Client, ClientBorrowMut},
    driver::codec::Response,
    error::Error,
    iter::AsyncLendingIterator,
    query::Query,
    statement::Statement,
    zero_params,
};

pub trait r#Copy: Query + ClientBorrowMut {
    fn send_one_way<F>(&self, func: F) -> Result<(), Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<(), Error>;
}

impl r#Copy for Client {
    #[inline]
    fn send_one_way<F>(&self, func: F) -> Result<(), Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<(), Error>,
    {
        self.tx.send_one_way(func)
    }
}

pub struct CopyIn<'a, C>
where
    C: r#Copy + Send,
{
    client: &'a mut C,
    res: Option<Response>,
}

impl<C> Drop for CopyIn<'_, C>
where
    C: r#Copy + Send,
{
    fn drop(&mut self) {
        // when response is not taken on drop it means the progress is aborted before finish.
        // cancel the copy in this case
        if self.res.is_some() {
            self.do_cancel();
        }
    }
}

impl<'a, C> CopyIn<'a, C>
where
    C: r#Copy + Send,
{
    pub fn new(client: &'a mut C, stmt: &Statement) -> impl Future<Output = Result<Self, Error>> + Send {
        // marker check to ensure exclusive borrowing Client. see ClientBorrowMut for detail
        let _cli = client._borrow_mut();

        let res = client._send_encode_query(stmt.bind(zero_params())).map(|(_, res)| res);

        async {
            let mut res = res?;
            match res.recv().await? {
                backend::Message::BindComplete => {}
                _ => return Err(Error::unexpected()),
            }

            match res.recv().await? {
                backend::Message::CopyInResponse(_) => {}
                _ => return Err(Error::unexpected()),
            }

            Ok(CopyIn { client, res: Some(res) })
        }
    }

    /// copy given buffer into [`Driver`] and send it to database in non blocking manner
    ///
    /// *. calling this api in rapid succession and/or supply huge buffer may result in high memory consumption.
    /// consider rate limiting the progress with small chunk of buffer and/or using smart pointers for throughput
    /// counting
    ///
    /// [`Driver`]: crate::driver::Driver
    pub fn copy(&mut self, item: impl Buf) -> Result<(), Error> {
        let data = frontend::CopyData::new(item)?;
        self.client.send_one_way(|buf| {
            data.write(buf);
            Ok(())
        })
    }

    /// finish copy in and return how many rows are affected
    pub async fn finish(mut self) -> Result<u64, Error> {
        self.client.send_one_way(|buf| {
            frontend::copy_done(buf);
            frontend::sync(buf);
            Ok(())
        })?;
        self.res.take().unwrap().try_into_row_affected().await
    }

    fn do_cancel(&mut self) {
        let _ = self.client.send_one_way(|buf| {
            frontend::copy_fail("", buf)?;
            frontend::sync(buf);
            Ok(())
        });
    }
}

pub struct CopyOut {
    res: Response,
}

impl CopyOut {
    pub fn new(cli: &impl Query, stmt: &Statement) -> impl Future<Output = Result<Self, Error>> + Send {
        let res = cli._send_encode_query(stmt.bind(zero_params())).map(|(_, res)| res);

        async {
            let mut res = res?;

            match res.recv().await? {
                backend::Message::BindComplete => {}
                _ => return Err(Error::unexpected()),
            }

            match res.recv().await? {
                backend::Message::CopyOutResponse(_) => {}
                _ => return Err(Error::unexpected()),
            }

            Ok(CopyOut { res })
        }
    }
}

impl AsyncLendingIterator for CopyOut {
    type Ok<'i>
        = Bytes
    where
        Self: 'i;
    type Err = Error;

    async fn try_next(&mut self) -> Result<Option<Self::Ok<'_>>, Self::Err> {
        match self.res.recv().await? {
            backend::Message::CopyData(body) => Ok(Some(body.into_bytes())),
            backend::Message::CopyDone => Ok(None),
            _ => Err(Error::unexpected()),
        }
    }
}
