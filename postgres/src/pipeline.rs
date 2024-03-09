use core::ops::Range;

use std::collections::VecDeque;

use postgres_protocol::message::{backend, frontend};
use postgres_types::BorrowToSql;
use xitca_io::bytes::BytesMut;

use super::{
    client::Client,
    column::Column,
    driver::Response,
    error::Error,
    iter::{slice_iter, AsyncLendingIterator},
    row::Row,
    statement::Statement,
    ToSql,
};

/// A pipelined sql query type. It lazily batch queries into local buffer and try to send it
/// with the least amount of syscall when pipeline starts.
///
/// # Examples
/// ```rust
/// use xitca_postgres::{AsyncLendingIterator, Client, pipeline::Pipeline};
///
/// async fn pipeline(client: &Client) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
///     let statement = client.prepare("SELECT * FROM public.users", &[]).await?;
///
///     let mut pipe = Pipeline::new();
///     pipe.query(statement.as_ref(), &[])?;
///     pipe.query_raw::<[i32; 0]>(statement.as_ref(), [])?;
///
///     let mut res = client.pipeline(pipe).await?;
///
///     while let Some(mut item) = res.try_next().await? {
///         while let Some(row) = item.try_next().await? {
///             let _: u32 = row.get("id");
///         }
///     }
///
///     Ok(())
/// }
/// ```
pub struct Pipeline<'a, const SYNC_MODE: bool = true> {
    pub(crate) columns: VecDeque<&'a [Column]>,
    // how many SYNC message we are sending to database.
    // it determines when the driver would shutdown the pipeline.
    pub(crate) sync_count: usize,
    pub(crate) buf: BytesMut,
}

fn _assert_pipe_send() {
    crate::_assert_send2::<Pipeline<'_>>();
}

impl Pipeline<'_, true> {
    /// start a new pipeline.
    ///
    /// pipeline is sync by default. which means every query inside is considered separate binding
    /// and the pipeline is transparent to database server. the pipeline only happen on socket
    /// transport where minimal amount of syscall is needed.
    ///
    /// for more relaxed [Pipeline Mode][libpq_link] see [Client::pipeline_unsync] api.
    ///
    /// [libpq_link]: https://www.postgresql.org/docs/current/libpq-pipeline-mode.html
    pub fn new() -> Self {
        Self {
            columns: VecDeque::new(),
            sync_count: 0,
            buf: BytesMut::new(),
        }
    }
}

impl Pipeline<'_, false> {
    /// start a new un-sync pipeline.
    ///
    /// in un-sync mode pipeline treat all queries inside as one single binding and database server
    /// can see them as no sync point in between which can result in potential performance gain.
    ///
    /// it behaves the same on transportation level as [Client::pipeline] where minimal amount
    /// of socket syscall is needed.
    #[inline]
    pub fn unsync() -> Self {
        Self {
            columns: VecDeque::new(),
            sync_count: 0,
            buf: BytesMut::new(),
        }
    }
}

impl<'a, const SYNC_MODE: bool> Pipeline<'a, SYNC_MODE> {
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
            .map(|_| {
                self.columns.push_back(stmt.columns());
                if SYNC_MODE {
                    self.sync_count += 1;
                }
            })
            .map_err(|e| {
                // revert back to last pipelined query when encoding error occurred.
                self.buf.truncate(len);
                e
            })
    }
}

impl Client {
    /// execute the pipeline.
    pub async fn pipeline<'a, const SYNC_MODE: bool>(
        &self,
        mut pipe: Pipeline<'a, SYNC_MODE>,
    ) -> Result<PipelineStream<'a>, Error> {
        if pipe.buf.is_empty() {
            return Ok(PipelineStream {
                res: Response::no_op(),
                columns: VecDeque::new(),
                ranges: Vec::new(),
            });
        }

        if !SYNC_MODE {
            pipe.sync_count += 1;
            frontend::sync(&mut pipe.buf);
        }

        self.pipeline_buf(pipe.sync_count, pipe.buf, pipe.columns).await
    }

    pub(crate) async fn _pipeline<'a, const SYNC_MODE: bool>(
        &self,
        sync_count: &mut usize,
        mut buf: BytesMut,
    ) -> Result<Response, Error> {
        assert!(!buf.is_empty());

        if !SYNC_MODE {
            *sync_count += 1;
            frontend::sync(&mut buf);
        }

        self.tx.send_multi(*sync_count, buf).await
    }

    pub(crate) async fn pipeline_buf<'a>(
        &self,
        sync_count: usize,
        buf: BytesMut,
        columns: VecDeque<&'a [Column]>,
    ) -> Result<PipelineStream<'a>, Error> {
        self.tx.send_multi(sync_count, buf).await.map(|res| PipelineStream {
            res,
            columns: columns,
            ranges: Vec::new(),
        })
    }
}

/// streaming response of pipeline.
/// impls [AsyncIterator] trait and can be collected asynchronously.
pub struct PipelineStream<'a> {
    pub(crate) res: Response,
    pub(crate) columns: VecDeque<&'a [Column]>,
    pub(crate) ranges: Vec<Option<Range<usize>>>,
}

impl<'a> AsyncLendingIterator for PipelineStream<'a> {
    type Ok<'i> = PipelineItem<'i, 'a> where Self: 'i;
    type Err = Error;

    async fn try_next(&mut self) -> Result<Option<Self::Ok<'_>>, Self::Err> {
        while !self.columns.is_empty() {
            match self.res.recv().await? {
                backend::Message::BindComplete => {
                    return Ok(Some(PipelineItem {
                        finished: false,
                        stream: self,
                        rows_affected: 0,
                    }));
                }
                backend::Message::DataRow(_) | backend::Message::CommandComplete(_) => {
                    // last PipelineItem dropped before finish. do some catch up until next
                    // item arrives.
                }
                backend::Message::ReadyForQuery(_) => {}
                _ => return Err(Error::UnexpectedMessage),
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
/// impls [AsyncIterator] and can be used to collect [Row] from item.
pub struct PipelineItem<'a, 'c> {
    finished: bool,
    stream: &'a mut PipelineStream<'c>,
    rows_affected: u64,
}

impl PipelineItem<'_, '_> {
    /// return the number of rows affected by certain query in pipeline.
    pub fn rows_affected(&self) -> u64 {
        self.rows_affected
    }
}

impl AsyncLendingIterator for PipelineItem<'_, '_> {
    type Ok<'i> = Row<'i> where Self: 'i;
    type Err = Error;

    async fn try_next(&mut self) -> Result<Option<Self::Ok<'_>>, Self::Err> {
        while !self.finished {
            match self.stream.res.recv().await? {
                backend::Message::DataRow(body) => {
                    let columns = self
                        .stream
                        .columns
                        .front()
                        .expect("PipelineItem must not overflow PipelineStream's columns array");
                    return Row::try_new(columns, body, &mut self.stream.ranges).map(Some);
                }
                backend::Message::CommandComplete(body) => {
                    self.finished = true;
                    self.rows_affected = crate::query::decode::body_to_affected_rows(&body)?;
                }
                _ => return Err(Error::UnexpectedMessage),
            }
        }

        Ok(None)
    }
}

impl Drop for PipelineItem<'_, '_> {
    fn drop(&mut self) {
        self.stream.columns.pop_front();
    }
}
