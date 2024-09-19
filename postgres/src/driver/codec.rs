use core::{
    future::{poll_fn, Future},
    task::{ready, Context, Poll},
};

use std::sync::Arc;

use postgres_protocol::message::{backend, frontend};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use xitca_io::bytes::BytesMut;

use crate::{
    error::{DbError, DriverDownReceiving, Error, InvalidParamCount},
    prepare::Prepare,
    query::{RowSimpleStream, RowStream, RowStreamGuarded},
    statement::{Statement, StatementGuarded, StatementUnnamed},
    types::{BorrowToSql, IsNull, Type},
};

use super::DriverTx;

pub(super) fn request_pair(msg_count: usize) -> (ResponseSender, Response) {
    let (tx, rx) = unbounded_channel();
    (
        ResponseSender { tx, msg_count },
        Response {
            rx,
            buf: BytesMut::new(),
        },
    )
}

pub struct Response {
    rx: ResponseReceiver,
    buf: BytesMut,
}

impl Response {
    pub(crate) fn recv(&mut self) -> impl Future<Output = Result<backend::Message, Error>> + Send + '_ {
        poll_fn(|cx| self.poll_recv(cx))
    }

    pub(crate) fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Result<backend::Message, Error>> {
        if self.buf.is_empty() {
            self.buf = ready!(self.rx.poll_recv(cx)).ok_or_else(|| DriverDownReceiving)?;
        }

        let res = match backend::Message::parse(&mut self.buf)?.expect("must not parse message from empty buffer.") {
            backend::Message::ErrorResponse(body) => {
                let e = DbError::parse(&mut body.fields())?;
                Err(e.into())
            }
            msg => Ok(msg),
        };

        Poll::Ready(res)
    }

    pub(crate) fn try_into_row_affected(mut self) -> impl Future<Output = Result<u64, Error>> + Send {
        let mut rows = 0;
        poll_fn(move |cx| {
            ready!(self.poll_try_into_ready(&mut rows, cx))?;
            Poll::Ready(Ok(rows))
        })
    }

    pub(crate) fn poll_try_into_ready(&mut self, rows: &mut u64, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        loop {
            match ready!(self.poll_recv(cx))? {
                backend::Message::BindComplete
                | backend::Message::DataRow(_)
                | backend::Message::EmptyQueryResponse => {}
                backend::Message::CommandComplete(body) => {
                    *rows = body_to_affected_rows(&body)?;
                }
                backend::Message::ReadyForQuery(_) => return Poll::Ready(Ok(())),
                _ => return Poll::Ready(Err(Error::unexpected())),
            }
        }
    }
}

// Extract the number of rows affected.
pub(crate) fn body_to_affected_rows(body: &backend::CommandCompleteBody) -> Result<u64, Error> {
    body.tag()
        .map_err(|_| Error::todo())
        .map(|r| r.rsplit(' ').next().unwrap().parse().unwrap_or(0))
}

#[derive(Debug)]
pub(crate) struct ResponseSender {
    tx: UnboundedSender<BytesMut>,
    msg_count: usize,
}

pub(super) enum SenderState {
    Continue,
    Finish,
}

impl ResponseSender {
    pub(super) fn send(&mut self, msg: BytesMut, complete: bool) -> SenderState {
        debug_assert!(self.msg_count > 0);

        let _ = self.tx.send(msg);

        if complete {
            self.msg_count -= 1;
        }

        if self.msg_count == 0 {
            SenderState::Finish
        } else {
            SenderState::Continue
        }
    }
}

// TODO: remove this lint.
#[allow(dead_code)]
pub(super) type ResponseReceiver = UnboundedReceiver<BytesMut>;

pub enum ResponseMessage {
    Normal { buf: BytesMut, complete: bool },
    Async(backend::Message),
}

impl ResponseMessage {
    pub(crate) fn try_from_buf(buf: &mut BytesMut) -> Result<Option<Self>, Error> {
        let mut idx = 0;
        let mut complete = false;

        loop {
            let slice = &buf[idx..];
            let Some(header) = backend::Header::parse(slice)? else {
                break;
            };
            let len = header.len() as usize + 1;

            if slice.len() < len {
                break;
            }

            match header.tag() {
                backend::NOTICE_RESPONSE_TAG | backend::NOTIFICATION_RESPONSE_TAG | backend::PARAMETER_STATUS_TAG => {
                    if idx == 0 {
                        // TODO:
                        // PagedBytesMut should never expose underlying BytesMut type as reference.
                        // this is needed because postgres-protocol is an external crate.
                        let message = backend::Message::parse(buf)?.unwrap();
                        return Ok(Some(ResponseMessage::Async(message)));
                    }

                    break;
                }
                tag => {
                    idx += len;
                    if matches!(tag, backend::READY_FOR_QUERY_TAG) {
                        complete = true;
                        break;
                    }
                }
            }
        }

        if idx == 0 {
            Ok(None)
        } else {
            Ok(Some(ResponseMessage::Normal {
                buf: buf.split_to(idx),
                complete,
            }))
        }
    }
}

/// super trait to constraint Self and associated types' trait bounds.
pub trait AsParams: IntoIterator<IntoIter: ExactSizeIterator, Item: BorrowToSql> {}

impl<I> AsParams for I
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: BorrowToSql,
{
}

mod sealed {
    pub trait Sealed {}
}

/// trait for generic over how to encode a query.
/// this trait can not be implement by library user.
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not impl Encode trait",
    label = "query statement argument must be types implement Encode trait",
    note = "consider using the types listed below that implementing Encode trait"
)]
pub trait Encode: sealed::Sealed + Sized + Copy {
    type RowStream<'r>
    where
        Self: 'r;

    fn encode<I, const SYNC_MODE: bool>(self, params: I, buf: &mut BytesMut) -> Result<(), Error>
    where
        I: AsParams;

    fn row_stream<'r>(self, res: Response) -> Self::RowStream<'r>
    where
        Self: 'r;
}

impl sealed::Sealed for &Statement {}

impl Encode for &Statement {
    type RowStream<'r> = RowStream<'r> where Self: 'r;

    fn encode<I, const SYNC_MODE: bool>(self, params: I, buf: &mut BytesMut) -> Result<(), Error>
    where
        I: AsParams,
    {
        encode_bind(self.name(), self.params(), params, "", buf)?;
        frontend::execute("", 0, buf)?;

        if SYNC_MODE {
            frontend::sync(buf);
        }

        Ok(())
    }

    #[inline]
    fn row_stream<'r>(self, res: Response) -> Self::RowStream<'r>
    where
        Self: 'r,
    {
        RowStream::new(res, self.columns())
    }
}

impl sealed::Sealed for &Arc<Statement> {}

impl Encode for &Arc<Statement> {
    type RowStream<'r> = <&'r Statement as Encode>::RowStream<'r> where Self: 'r;

    #[inline]
    fn encode<I, const SYNC_MODE: bool>(self, params: I, buf: &mut BytesMut) -> Result<(), Error>
    where
        I: AsParams,
    {
        <&Statement>::encode::<_, SYNC_MODE>(self, params, buf)
    }

    #[inline]
    fn row_stream<'r>(self, res: Response) -> Self::RowStream<'r>
    where
        Self: 'r,
    {
        <&Statement>::row_stream(self, res)
    }
}

impl<C> sealed::Sealed for &StatementGuarded<'_, C> where C: Prepare {}

impl<C> Encode for &StatementGuarded<'_, C>
where
    C: Prepare,
{
    type RowStream<'r> = <&'r Statement as Encode>::RowStream<'r> where Self: 'r;

    #[inline]
    fn encode<I, const SYNC_MODE: bool>(self, params: I, buf: &mut BytesMut) -> Result<(), Error>
    where
        I: AsParams,
    {
        <&Statement>::encode::<_, SYNC_MODE>(self, params, buf)
    }

    #[inline]
    fn row_stream<'r>(self, res: Response) -> Self::RowStream<'r>
    where
        Self: 'r,
    {
        <&Statement>::row_stream(self, res)
    }
}

impl<C> sealed::Sealed for StatementUnnamed<'_, C> where C: Prepare {}

impl<'a, C> Encode for StatementUnnamed<'a, C>
where
    C: Prepare,
{
    type RowStream<'r> = RowStreamGuarded<'r, C> where 'a: 'r;

    #[inline]
    fn encode<I, const SYNC_MODE: bool>(self, params: I, buf: &mut BytesMut) -> Result<(), Error>
    where
        I: AsParams,
    {
        let Self { stmt, types, .. } = self;
        frontend::parse("", stmt, types.iter().map(Type::oid), buf)?;
        encode_bind("", types, params, "", buf)?;
        frontend::describe(b'S', "", buf)?;
        frontend::execute("", 0, buf)?;
        if SYNC_MODE {
            frontend::sync(buf);
        }
        Ok(())
    }

    #[inline]
    fn row_stream<'r>(self, res: Response) -> Self::RowStream<'r>
    where
        Self: 'r,
    {
        RowStreamGuarded::new(res, self.cli)
    }
}

impl sealed::Sealed for &str {}

impl Encode for &str {
    type RowStream<'r> = RowSimpleStream where Self: 'r;

    #[inline]
    fn encode<I, const SYNC_MODE: bool>(self, _: I, buf: &mut BytesMut) -> Result<(), Error>
    where
        I: AsParams,
    {
        frontend::query(self, buf).map_err(Into::into)
    }

    #[inline]
    fn row_stream<'r>(self, res: Response) -> Self::RowStream<'r>
    where
        Self: 'r,
    {
        RowSimpleStream::new(res, Vec::new())
    }
}

impl sealed::Sealed for &String {}

impl Encode for &String {
    type RowStream<'r> = <&'r str as Encode>::RowStream<'r> where Self: 'r;

    #[inline]
    fn encode<I, const SYNC_MODE: bool>(self, params: I, buf: &mut BytesMut) -> Result<(), Error>
    where
        I: AsParams,
    {
        self.as_str().encode::<_, SYNC_MODE>(params, buf)
    }

    #[inline]
    fn row_stream<'r>(self, res: Response) -> Self::RowStream<'r>
    where
        Self: 'r,
    {
        self.as_str().row_stream(res)
    }
}

#[cfg(feature = "compat")]
const _: () = {
    use crate::statement::compat::StatementGuarded;

    impl<C> sealed::Sealed for &StatementGuarded<C> where C: Prepare {}

    impl<C> Encode for &StatementGuarded<C>
    where
        C: Prepare,
    {
        type RowStream<'r> = RowStream<'r> where Self: 'r;

        #[inline]
        fn encode<I, const SYNC_MODE: bool>(self, params: I, buf: &mut BytesMut) -> Result<(), Error>
        where
            I: AsParams,
        {
            <&Statement>::encode::<_, SYNC_MODE>(self, params, buf)
        }

        #[inline]
        fn row_stream<'r>(self, res: Response) -> Self::RowStream<'r>
        where
            Self: 'r,
        {
            <&Statement>::row_stream(self, res)
        }
    }
};

pub(crate) fn send_encode_query<S, I>(tx: &DriverTx, stmt: S, params: I) -> Result<Response, Error>
where
    S: Encode,
    I: AsParams,
{
    tx.send(|buf| stmt.encode::<_, true>(params, buf))
}

pub(crate) fn send_encode_portal_create<I>(
    tx: &DriverTx,
    name: &str,
    stmt: &Statement,
    params: I,
) -> Result<Response, Error>
where
    I: ExactSizeIterator,
    I::Item: BorrowToSql,
{
    tx.send(|buf| {
        encode_bind(stmt.name(), stmt.params(), params, name, buf)?;
        frontend::sync(buf);
        Ok(())
    })
}

pub(crate) fn send_encode_portal_query(tx: &DriverTx, name: &str, max_rows: i32) -> Result<Response, Error> {
    tx.send(|buf| {
        frontend::execute(name, max_rows, buf)?;
        frontend::sync(buf);
        Ok(())
    })
}

pub(crate) fn send_encode_portal_cancel(tx: &DriverTx, name: &str) -> Result<Response, Error> {
    send_cancel(b'P', tx, name)
}

pub(crate) fn send_encode_statement_create(
    tx: &DriverTx,
    name: &str,
    query: &str,
    types: &[Type],
) -> Result<Response, Error> {
    tx.send(|buf| {
        frontend::parse(name, query, types.iter().map(Type::oid), buf)?;
        frontend::describe(b'S', name, buf)?;
        frontend::sync(buf);
        Ok(())
    })
}

pub(crate) fn send_encode_statement_cancel(tx: &DriverTx, name: &str) -> Result<Response, Error> {
    send_cancel(b'S', tx, name)
}

fn send_cancel(variant: u8, tx: &DriverTx, name: &str) -> Result<Response, Error> {
    tx.send(|buf| {
        frontend::close(variant, name, buf)?;
        frontend::sync(buf);
        Ok(())
    })
}

fn encode_bind<P>(stmt: &str, types: &[Type], params: P, portal: &str, buf: &mut BytesMut) -> Result<(), Error>
where
    P: AsParams,
{
    let params = params.into_iter();
    if params.len() != types.len() {
        return Err(Error::from(InvalidParamCount {
            expected: types.len(),
            params: params.len(),
        }));
    }

    let params = params.zip(types).collect::<Vec<_>>();

    frontend::bind(
        portal,
        stmt,
        params.iter().map(|(p, ty)| p.borrow_to_sql().encode_format(ty) as _),
        params.iter(),
        |(param, ty), buf| {
            param
                .borrow_to_sql()
                .to_sql_checked(ty, buf)
                .map(|is_null| match is_null {
                    IsNull::No => postgres_protocol::IsNull::No,
                    IsNull::Yes => postgres_protocol::IsNull::Yes,
                })
        },
        Some(1),
        buf,
    )
    .map_err(|e| match e {
        frontend::BindError::Conversion(e) => Error::from(e),
        frontend::BindError::Serialization(e) => Error::from(e),
    })
}
