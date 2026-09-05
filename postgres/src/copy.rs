use xitca_io::bytes::{Buf, Bytes, BytesMut};

use super::{
    client::{Client, ClientBorrow, ClientBorrowMut},
    driver::codec::Response,
    error::Error,
    iter::AsyncLendingIterator,
    protocol::message::{backend, frontend},
    statement::Statement,
};

pub trait r#Copy: ClientBorrowMut {
    /// copy messages are issued from sync methods that can not wait for a queue slot. copy data
    /// is rate limited by the caller and copy done/fail complete a copy already in progress, so
    /// none of them take part in backpressure.
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
        self.tx.send_one_way_unbounded(func)
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
    pub async fn new(client: &'a mut C, stmt: &Statement) -> Result<Self, Error> {
        // marker check to ensure exclusive borrowing Client. see ClientBorrowMut for detail
        let res = client
            .borrow_cli_mut()
            .query_raw(stmt.bind_none())
            .await
            .map(|(_, res)| res);

        {
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
    pub async fn new(cli: &impl ClientBorrow, stmt: &Statement) -> Result<Self, Error> {
        let res = cli
            .borrow_cli_ref()
            .query_raw(stmt.bind_none())
            .await
            .map(|(_, res)| res);

        {
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
