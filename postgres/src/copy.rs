use core::future::Future;

use postgres_protocol::message::{backend, frontend};
use xitca_io::bytes::{Buf, Bytes};

use crate::AsyncLendingIterator;

use super::{client::Client, driver::codec::Response, error::Error, statement::Statement};

pub struct CopyIn<'a, C>
where
    C: r#Copy,
{
    client: &'a mut C,
    res: Option<Response>,
}

impl<C> Drop for CopyIn<'_, C>
where
    C: r#Copy,
{
    fn drop(&mut self) {
        // when response is not taken on drop it means the progress is aborted before finish.
        // cancel the copy in this case
        if self.res.is_some() {
            self.client.copy_in_cancel();
        }
    }
}

impl<'a, C> CopyIn<'a, C>
where
    C: r#Copy,
{
    pub(crate) async fn new(client: &'a mut C, stmt: &Statement) -> Result<Self, Error> {
        let mut res = client._encode_send(stmt)?;

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

    /// copy given buffer into [`Driver`] and send it to database in non blocking manner
    ///
    /// *. calling this api in rapid succession and/or supply huge buffer may result in high memory consumption.
    /// consider rate limiting the progress with small chunk of buffer and/or using smart pointers for throughput
    /// counting
    ///
    /// [`Driver`]: crate::driver::Driver
    pub fn copy(&mut self, item: impl Buf) -> Result<(), Error> {
        self.client.copy_in(item)
    }

    /// finish copy in and return how many rows are affected
    pub async fn finish(mut self) -> Result<u64, Error> {
        self.client.copy_in_finish()?;
        self.res.take().unwrap().try_into_row_affected().await
    }
}

pub trait r#Copy {
    fn copy_in<B>(&mut self, item: B) -> Result<(), Error>
    where
        B: Buf;

    fn copy_in_finish(&mut self) -> Result<(), Error>;

    fn copy_in_cancel(&mut self);

    fn _encode_send(&mut self, stmt: &Statement) -> Result<Response, Error>;
}

impl r#Copy for Client {
    fn copy_in<B>(&mut self, item: B) -> Result<(), Error>
    where
        B: Buf,
    {
        let data = frontend::CopyData::new(item)?;
        self.tx.send_one_way(|buf| {
            data.write(buf);
            Ok(())
        })
    }

    fn copy_in_finish(&mut self) -> Result<(), Error> {
        self.tx.send_one_way(|buf| {
            frontend::copy_done(buf);
            frontend::sync(buf);
            Ok(())
        })
    }

    fn copy_in_cancel(&mut self) {
        let _ = self.tx.send_one_way(|buf| {
            frontend::copy_fail("", buf)?;
            frontend::sync(buf);
            Ok(())
        });
    }

    fn _encode_send(&mut self, stmt: &Statement) -> Result<Response, Error> {
        self.send_encode::<[i32; 0]>(stmt, [])
    }
}

impl Client {
    /// start a copy in query
    pub async fn copy_in(&mut self, stmt: &Statement) -> Result<CopyIn<'_, Client>, Error> {
        CopyIn::new(self, stmt).await
    }

    /// start a copy out query
    pub fn copy_out(&self, stmt: &Statement) -> impl Future<Output = Result<CopyOut, Error>> + Send {
        let res = self.send_encode::<[i32; 0]>(stmt, []);

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

pub struct CopyOut {
    res: Response,
}

impl AsyncLendingIterator for CopyOut {
    type Ok<'i> = Bytes
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
