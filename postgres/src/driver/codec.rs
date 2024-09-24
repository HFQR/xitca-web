use core::{
    future::{poll_fn, Future},
    task::{ready, Context, Poll},
};

use std::sync::Arc;

use postgres_protocol::message::{backend, frontend};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use xitca_io::bytes::BytesMut;

use crate::{
    column::Column,
    error::{DriverDownReceiving, Error, InvalidParamCount},
    prepare::Prepare,
    query::{RowSimpleStream, RowStream, RowStreamGuarded},
    statement::{Statement, StatementGuarded, StatementUnnamed},
    types::{BorrowToSql, IsNull, ToSql, Type},
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

#[derive(Debug)]
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
            backend::Message::ErrorResponse(body) => Err(Error::db(body.fields())),
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
                | backend::Message::RowDescription(_)
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

/// traits for converting typed parameters into exact sized iterator where it yields
/// item can be converted in binary format of postgres type.
pub trait IntoParams {
    fn into_params(self) -> impl ExactSizeIterator<Item: BorrowToSql>;
}

impl<I, const N: usize> IntoParams for [I; N]
where
    I: ToSql,
{
    #[inline]
    fn into_params(self) -> impl ExactSizeIterator<Item: BorrowToSql> {
        self.into_iter()
    }
}

impl<I> IntoParams for Vec<I>
where
    I: ToSql,
{
    #[inline]
    fn into_params(self) -> impl ExactSizeIterator<Item: BorrowToSql> {
        self.into_iter()
    }
}

impl IntoParams for &[&(dyn ToSql + Sync)] {
    #[inline]
    fn into_params(self) -> impl ExactSizeIterator<Item: BorrowToSql> {
        self.iter().cloned()
    }
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
pub trait Encode: sealed::Sealed + Sized {
    /// output type defines how a potential async row streaming type should be constructed.
    /// certain state from the encode type may need to be passed for constructing the stream
    type Output<'o>: IntoStream
    where
        Self: 'o;

    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o;
}

/// trait for generic over how to construct an async stream rows
pub trait IntoStream: sealed::Sealed + Sized {
    type RowStream<'r>
    where
        Self: 'r;

    fn into_stream<'r>(self, res: Response) -> Self::RowStream<'r>
    where
        Self: 'r;
}

impl<I> sealed::Sealed for (&Statement, I) {}

impl<I> Encode for (&Statement, I)
where
    I: IntoParams,
{
    type Output<'o>
        = &'o [Column]
    where
        Self: 'o;

    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        let (stmt, params) = self;
        encode_bind(stmt.name(), stmt.params(), params, "", buf)?;
        frontend::execute("", 0, buf)?;

        if SYNC_MODE {
            frontend::sync(buf);
        }

        Ok(stmt.columns())
    }
}

impl sealed::Sealed for &[Column] {}

impl IntoStream for &[Column] {
    type RowStream<'r>
        = RowStream<'r>
    where
        Self: 'r;

    #[inline]
    fn into_stream<'r>(self, res: Response) -> Self::RowStream<'r>
    where
        Self: 'r,
    {
        RowStream::new(res, self)
    }
}

impl<I> sealed::Sealed for (&Arc<Statement>, I) {}

impl<I> Encode for (&Arc<Statement>, I)
where
    I: IntoParams,
{
    type Output<'o>
        = &'o [Column]
    where
        Self: 'o;

    #[inline]
    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        let (stmt, params) = self;
        <(&Statement, I)>::encode::<SYNC_MODE>((stmt, params), buf)
    }
}

impl<C, I> sealed::Sealed for (&StatementGuarded<'_, C>, I) where C: Prepare {}

impl<C, I> Encode for (&StatementGuarded<'_, C>, I)
where
    C: Prepare,
    I: IntoParams,
{
    type Output<'o>
        = &'o [Column]
    where
        Self: 'o;

    #[inline]
    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        let (stmt, params) = self;
        <(&Statement, I)>::encode::<SYNC_MODE>((stmt, params), buf)
    }
}

impl<C, I> sealed::Sealed for (StatementUnnamed<'_, C>, I) where C: Prepare {}

impl<C, I> Encode for (StatementUnnamed<'_, C>, I)
where
    C: Prepare,
    I: IntoParams,
{
    type Output<'o>
        = IntoRowStreamGuard<'o, C>
    where
        Self: 'o;

    #[inline]
    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        let (StatementUnnamed { stmt, types, cli }, params) = self;
        frontend::parse("", stmt, types.iter().map(Type::oid), buf)?;
        encode_bind("", types, params, "", buf)?;
        frontend::describe(b'S', "", buf)?;
        frontend::execute("", 0, buf)?;
        if SYNC_MODE {
            frontend::sync(buf);
        }
        Ok(IntoRowStreamGuard(cli))
    }
}

pub struct IntoRowStreamGuard<'a, C>(&'a C);

impl<C> sealed::Sealed for IntoRowStreamGuard<'_, C> {}

impl<C> IntoStream for IntoRowStreamGuard<'_, C>
where
    C: Prepare,
{
    type RowStream<'r>
        = RowStreamGuarded<'r, C>
    where
        Self: 'r;

    #[inline]
    fn into_stream<'r>(self, res: Response) -> Self::RowStream<'r>
    where
        Self: 'r,
    {
        RowStreamGuarded::new(res, self.0)
    }
}

impl sealed::Sealed for &str {}

impl Encode for &str {
    type Output<'o>
        = Vec<Column>
    where
        Self: 'o;

    #[inline]
    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        frontend::query(self, buf)?;
        Ok(Vec::new())
    }
}

impl sealed::Sealed for Vec<Column> {}

impl IntoStream for Vec<Column> {
    type RowStream<'r>
        = RowSimpleStream
    where
        Self: 'r;

    #[inline]
    fn into_stream<'r>(self, res: Response) -> Self::RowStream<'r>
    where
        Self: 'r,
    {
        RowSimpleStream::new(res, self)
    }
}

impl sealed::Sealed for &String {}

impl Encode for &String {
    type Output<'o>
        = Vec<Column>
    where
        Self: 'o;

    #[inline]
    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        self.as_str().encode::<SYNC_MODE>(buf)
    }
}

#[cfg(feature = "compat")]
const _: () = {
    use crate::statement::compat::StatementGuarded;

    impl<C, I> sealed::Sealed for (&StatementGuarded<C>, I) where C: Prepare {}

    impl<C, I> Encode for (&StatementGuarded<C>, I)
    where
        C: Prepare,
        I: IntoParams,
    {
        type Output<'o>
            = &'o [Column]
        where
            Self: 'o;

        #[inline]
        fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
        where
            Self: 'o,
        {
            let (stmt, params) = self;
            <(&Statement, I)>::encode::<SYNC_MODE>((stmt, params), buf)
        }
    }
};

/// type for case where no row stream can be created.
/// the api caller should never call into_stream method from this type.
pub struct NoOpIntoRowStream;

impl sealed::Sealed for NoOpIntoRowStream {}

impl IntoStream for NoOpIntoRowStream {
    type RowStream<'r>
        = RowStream<'r>
    where
        Self: 'r;

    fn into_stream<'r>(self, _: Response) -> Self::RowStream<'r>
    where
        Self: 'r,
    {
        unreachable!("no row stream can be generated from no op row stream constructor")
    }
}

pub(crate) struct StatementCreate<'a> {
    pub(crate) name: &'a str,
    pub(crate) query: &'a str,
    pub(crate) types: &'a [Type],
}

impl sealed::Sealed for StatementCreate<'_> {}

impl Encode for StatementCreate<'_> {
    type Output<'o>
        = NoOpIntoRowStream
    where
        Self: 'o;

    #[inline]
    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        let Self { name, query, types } = self;
        frontend::parse(name, query, types.iter().map(Type::oid), buf)?;
        frontend::describe(b'S', name, buf)?;
        frontend::sync(buf);
        Ok(NoOpIntoRowStream)
    }
}

pub(crate) struct StatementCancel<'a> {
    pub(crate) name: &'a str,
}

impl sealed::Sealed for StatementCancel<'_> {}

impl Encode for StatementCancel<'_> {
    type Output<'o>
        = NoOpIntoRowStream
    where
        Self: 'o;

    #[inline]
    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        let Self { name } = self;
        frontend::close(b'S', name, buf)?;
        frontend::sync(buf);
        Ok(NoOpIntoRowStream)
    }
}

pub(crate) struct PortalCreate<'a> {
    pub(crate) name: &'a str,
    pub(crate) stmt: &'a str,
    pub(crate) types: &'a [Type],
}

impl<I> sealed::Sealed for (PortalCreate<'_>, I) {}

impl<I> Encode for (PortalCreate<'_>, I)
where
    I: IntoParams,
{
    type Output<'o>
        = NoOpIntoRowStream
    where
        Self: 'o;

    #[inline]
    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        let (PortalCreate { name, stmt, types }, params) = self;
        encode_bind(stmt, types, params, name, buf)?;
        frontend::sync(buf);
        Ok(NoOpIntoRowStream)
    }
}

pub(crate) struct PortalCancel<'a> {
    pub(crate) name: &'a str,
}

impl sealed::Sealed for PortalCancel<'_> {}

impl Encode for PortalCancel<'_> {
    type Output<'o>
        = NoOpIntoRowStream
    where
        Self: 'o;

    #[inline]
    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        frontend::close(b'P', self.name, buf)?;
        frontend::sync(buf);
        Ok(NoOpIntoRowStream)
    }
}

#[derive(Clone, Copy)]
pub struct PortalQuery<'a> {
    pub(crate) name: &'a str,
    pub(crate) columns: &'a [Column],
    pub(crate) max_rows: i32,
}

impl sealed::Sealed for PortalQuery<'_> {}

impl Encode for PortalQuery<'_> {
    type Output<'o>
        = &'o [Column]
    where
        Self: 'o;

    #[inline]
    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        let Self {
            name,
            max_rows,
            columns,
        } = self;
        frontend::execute(name, max_rows, buf)?;
        frontend::sync(buf);
        Ok(columns)
    }
}

pub(crate) fn send_encode_query<'a, S>(tx: &DriverTx, stmt: S) -> Result<(S::Output<'a>, Response), Error>
where
    S: Encode + 'a,
{
    tx.send(|buf| stmt.encode::<true>(buf))
}

fn encode_bind<P>(stmt: &str, types: &[Type], params: P, portal: &str, buf: &mut BytesMut) -> Result<(), Error>
where
    P: IntoParams,
{
    let params = params.into_params();
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
