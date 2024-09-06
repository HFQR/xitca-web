//! explicit pipelining module
//!
//! this crate supports "implicit" pipeline like [`tokio-postgres`] does and explicit pipeline is an optional addition.
//!
//! making pipelined queries with explicit types and apis has following benefits:
//! - reduced lock contention. explicit pipeline only lock client once when executed regardless query count
//! - flexible transform between sync and un-sync pipeline. See [Pipeline::new] for detail
//! - ordered response handling with a single stream type. reduce memory footprint
//!
//! [`tokio-postgres`]: https://docs.rs/tokio-postgres/latest/tokio_postgres/#pipelining
use core::ops::{Deref, DerefMut, Range};

use postgres_protocol::message::{backend, frontend};
use xitca_io::bytes::BytesMut;

use super::{
    client::Client,
    column::Column,
    driver::codec::Response,
    error::Error,
    iter::{slice_iter, AsyncLendingIterator},
    row::Row,
    statement::Statement,
    BorrowToSql, ToSql,
};

/// A pipelined sql query type. It lazily batch queries into local buffer and try to send it
/// with the least amount of syscall when pipeline starts.
///
/// # Examples
/// ```rust
/// use xitca_postgres::{AsyncLendingIterator, Client, pipeline::Pipeline};
///
/// async fn pipeline(client: &Client) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
///     // prepare a statement that will be called repeatedly.
///     // it can be a collection of statements that will be called in iteration.
///     let statement = client.prepare("SELECT * FROM public.users", &[]).await?;
///
///     // create a new pipeline.
///     let mut pipe = Pipeline::new();
///
///     // pipeline can encode multiple queries.
///     pipe.query(statement.as_ref(), &[])?;
///     pipe.query_raw::<[i32; 0]>(statement.as_ref(), [])?;
///
///     // execute the pipeline and on success a streaming response will be returned.
///     let mut res = client.pipeline(pipe)?;
///
///     // iterate through the query responses. the response order is the same as the order of
///     // queries encoded into pipeline with Pipeline::query_xxx api.
///     while let Some(mut item) = res.try_next().await? {
///         // every query can contain streaming rows.
///         while let Some(row) = item.try_next().await? {
///             let _: u32 = row.get("id");
///         }
///     }
///
///     Ok(())
/// }
/// ```
pub struct Pipeline<'a, B = Owned, const SYNC_MODE: bool = true> {
    pub(crate) columns: Vec<&'a [Column]>,
    // type for either owned or borrowed bytes buffer.
    pub(crate) buf: B,
}

/// borrowed bytes buffer supplied by api caller
pub struct Borrowed<'a>(&'a mut BytesMut);

/// owned bytes buffer created by [Pipeline]
pub struct Owned(BytesMut);

impl Deref for Borrowed<'_> {
    type Target = BytesMut;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl DerefMut for Borrowed<'_> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0
    }
}

impl Drop for Borrowed<'_> {
    fn drop(&mut self) {
        self.0.clear();
    }
}

impl Deref for Owned {
    type Target = BytesMut;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Owned {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<'a> From<Borrowed<'a>> for Owned {
    fn from(buf: Borrowed<'a>) -> Self {
        Self(BytesMut::from(buf.as_ref()))
    }
}

fn _assert_pipe_send() {
    crate::_assert_send2::<Pipeline<'_, Owned>>();
    crate::_assert_send2::<Pipeline<'_, Borrowed<'_>>>();
}

impl Pipeline<'_, Owned, true> {
    /// start a new pipeline.
    ///
    /// pipeline is sync by default. which means every query inside is considered separate binding
    /// and the pipeline is transparent to database server. the pipeline only happen on socket
    /// transport where minimal amount of syscall is needed.
    ///
    /// for more relaxed [Pipeline Mode][libpq_link] see [Pipeline::unsync] api.
    ///
    /// [libpq_link]: https://www.postgresql.org/docs/current/libpq-pipeline-mode.html
    #[inline]
    pub fn new() -> Self {
        Self::with_capacity(0)
    }

    /// start a new pipeline with given capacity.
    /// capacity represent how many queries will be contained by a single pipeline. a determined cap
    /// can possibly reduce memory reallocation when constructing the pipeline.
    pub fn with_capacity(cap: usize) -> Self {
        Self::_with_capacity(cap)
    }
}

impl Pipeline<'_, Owned, false> {
    /// start a new un-sync pipeline.
    ///
    /// in un-sync mode pipeline treat all queries inside as one single binding and database server
    /// can see them as no sync point in between which can result in potential performance gain.
    ///
    /// it behaves the same on transportation level as [Pipeline::new] where minimal amount
    /// of socket syscall is needed.
    #[inline]
    pub fn unsync() -> Self {
        Self::unsync_with_capacity(0)
    }

    /// start a new un-sync pipeline with given capacity.
    /// capacity represent how many queries will be contained by a single pipeline. a determined cap
    /// can possibly reduce memory reallocation when constructing the pipeline.
    pub fn unsync_with_capacity(cap: usize) -> Self {
        Self::_with_capacity(cap)
    }
}

impl<'a> Pipeline<'_, Borrowed<'a>, true> {
    /// start a new borrowed pipeline. pipeline will use borrowed bytes buffer to store encode messages
    /// before sending it to database.
    ///
    /// pipeline is sync by default. which means every query inside is considered separate binding
    /// and the pipeline is transparent to database server. the pipeline only happen on socket
    /// transport where minimal amount of syscall is needed.
    ///
    /// for more relaxed [Pipeline Mode][libpq_link] see [Pipeline::unsync_from_buf] api.
    ///
    /// [libpq_link]: https://www.postgresql.org/docs/current/libpq-pipeline-mode.html
    #[inline]
    pub fn from_buf(buf: &'a mut BytesMut) -> Self {
        Self::with_capacity_from_buf(0, buf)
    }

    /// start a new borrowed pipeline with given capacity.
    /// capacity represent how many queries will be contained by a single pipeline. a determined cap
    /// can possibly reduce memory reallocation when constructing the pipeline.
    #[inline]
    pub fn with_capacity_from_buf(cap: usize, buf: &'a mut BytesMut) -> Self {
        Self::_with_capacity_from_buf(cap, buf)
    }
}

impl<'a> Pipeline<'_, Borrowed<'a>, false> {
    /// start a new borrowed un-sync pipeline.
    ///
    /// in un-sync mode pipeline treat all queries inside as one single binding and database server
    /// can see them as no sync point in between which can result in potential performance gain.
    ///
    /// it behaves the same on transportation level as [Pipeline::from_buf] where minimal amount
    /// of socket syscall is needed.
    #[inline]
    pub fn unsync_from_buf(buf: &'a mut BytesMut) -> Self {
        Self::unsync_with_capacity_from_buf(0, buf)
    }

    /// start a new borrowed un-sync pipeline with given capacity.
    /// capacity represent how many queries will be contained by a single pipeline. a determined cap
    /// can possibly reduce memory reallocation when constructing the pipeline.
    #[inline]
    pub fn unsync_with_capacity_from_buf(cap: usize, buf: &'a mut BytesMut) -> Self {
        Self::_with_capacity_from_buf(cap, buf)
    }
}

impl<const SYNC_MODE: bool> Pipeline<'_, Owned, SYNC_MODE> {
    fn _with_capacity(cap: usize) -> Self {
        Self {
            columns: Vec::with_capacity(cap),
            buf: Owned(BytesMut::new()),
        }
    }
}

impl<'b, const SYNC_MODE: bool> Pipeline<'_, Borrowed<'b>, SYNC_MODE> {
    fn _with_capacity_from_buf(cap: usize, buf: &'b mut BytesMut) -> Self {
        debug_assert!(buf.is_empty(), "pipeline is borrowing potential polluted buffer");
        Self {
            columns: Vec::with_capacity(cap),
            buf: Borrowed(buf),
        }
    }
}

impl<'a, B, const SYNC_MODE: bool> Pipeline<'a, B, SYNC_MODE>
where
    B: DerefMut<Target = BytesMut>,
{
    /// pipelined version of [Client::query] and [Client::execute]
    #[inline]
    pub fn query(&mut self, stmt: &'a Statement, params: &[&(dyn ToSql + Sync)]) -> Result<(), Error> {
        self.query_raw(stmt, slice_iter(params))
    }

    /// pipelined version of [Client::query_raw] and [Client::execute_raw]
    pub fn query_raw<I>(&mut self, stmt: &'a Statement, params: I) -> Result<(), Error>
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        let params = params.into_iter();
        stmt.params_assert(&params);
        let len = self.buf.len();
        crate::query::encode::encode_maybe_sync::<_, SYNC_MODE>(&mut self.buf, stmt, params)
            .map(|_| self.columns.push(stmt.columns()))
            // revert back to last pipelined query when encoding error occurred.
            .inspect_err(|_| self.buf.truncate(len))
    }
}

impl Client {
    /// execute the pipeline.
    pub fn pipeline<'a, B, const SYNC_MODE: bool>(
        &self,
        mut pipe: Pipeline<'a, B, SYNC_MODE>,
    ) -> Result<PipelineStream<'a>, Error>
    where
        B: DerefMut<Target = BytesMut>,
    {
        let Pipeline { columns, ref mut buf } = pipe;
        self._pipeline::<SYNC_MODE, true>(&columns, buf)
            .map(|res| PipelineStream::new(res, columns))
    }

    pub(crate) fn _pipeline<const SYNC_MODE: bool, const ENCODE_SYNC: bool>(
        &self,
        columns: &[&[Column]],
        buf: &mut BytesMut,
    ) -> Result<Response, Error> {
        assert!(!buf.is_empty());

        let sync_count = if SYNC_MODE {
            columns.len()
        } else {
            if ENCODE_SYNC {
                frontend::sync(buf);
            }
            1
        };

        self.tx.send_multi_with(
            |b| {
                b.extend_from_slice(buf);
                Ok(())
            },
            sync_count,
        )
    }
}

/// streaming response of pipeline.
/// impl [AsyncLendingIterator] trait and can be collected asynchronously.
pub struct PipelineStream<'a> {
    res: Response,
    columns: Columns<'a>,
    ranges: Ranges,
}

impl<'a> PipelineStream<'a> {
    pub(crate) const fn new(res: Response, columns: Vec<&'a [Column]>) -> Self {
        Self {
            res,
            columns: Columns { columns, next: 0 },
            ranges: Vec::new(),
        }
    }
}

type Ranges = Vec<Range<usize>>;

struct Columns<'a> {
    columns: Vec<&'a [Column]>,
    next: usize,
}

impl<'a> Columns<'a> {
    // only move the cursor by one.
    // column references will be removed when pipeline stream is dropped.
    fn pop_front(&mut self) -> &'a [Column] {
        let off = self.next;
        self.next += 1;
        self.columns[off]
    }

    fn len(&self) -> usize {
        self.columns.len() - self.next
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<'a> AsyncLendingIterator for PipelineStream<'a> {
    type Ok<'i> = PipelineItem<'i> where Self: 'i;
    type Err = Error;

    async fn try_next(&mut self) -> Result<Option<Self::Ok<'_>>, Self::Err> {
        while !self.columns.is_empty() {
            match self.res.recv().await? {
                backend::Message::BindComplete => {
                    return Ok(Some(PipelineItem {
                        finished: false,
                        res: &mut self.res,
                        ranges: &mut self.ranges,
                        columns: self.columns.pop_front(),
                    }));
                }
                backend::Message::DataRow(_) | backend::Message::CommandComplete(_) => {
                    // last PipelineItem dropped before finish. do some catch up until next
                    // item arrives.
                }
                backend::Message::ReadyForQuery(_) => {}
                _ => return Err(Error::unexpected()),
            }
        }

        Ok(None)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.columns.len();
        (len, Some(len))
    }
}

/// streaming item of certain query inside pipeline's [PipelineStream].
/// impl [AsyncLendingIterator] and can be used to collect [Row] from item.
pub struct PipelineItem<'a> {
    finished: bool,
    res: &'a mut Response,
    ranges: &'a mut Ranges,
    columns: &'a [Column],
}

impl PipelineItem<'_> {
    /// collect rows affected by this pipelined query. [Row] information will be ignored.
    ///
    /// # Panic
    /// calling this method on an already finished PipelineItem will cause panic. PipelineItem is marked as finished
    /// when its [AsyncLendingIterator::try_next] method returns [Option::None]
    pub async fn row_affected(mut self) -> Result<u64, Error> {
        assert!(!self.finished, "PipelineItem has already finished");
        loop {
            match self.res.recv().await? {
                backend::Message::DataRow(_) => {}
                backend::Message::CommandComplete(body) => {
                    self.finished = true;
                    return crate::query::decode::body_to_affected_rows(&body);
                }
                _ => return Err(Error::unexpected()),
            }
        }
    }
}

impl AsyncLendingIterator for PipelineItem<'_> {
    type Ok<'i> = Row<'i> where Self: 'i;
    type Err = Error;

    async fn try_next(&mut self) -> Result<Option<Self::Ok<'_>>, Self::Err> {
        if !self.finished {
            match self.res.recv().await? {
                backend::Message::DataRow(body) => {
                    return Row::try_new(self.columns, body, self.ranges).map(Some);
                }
                backend::Message::CommandComplete(_) => self.finished = true,
                _ => return Err(Error::unexpected()),
            }
        }

        Ok(None)
    }
}
