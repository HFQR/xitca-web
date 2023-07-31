use core::{future::Future, ops::Range};

use std::collections::VecDeque;

use postgres_protocol::message::{backend, frontend};
use postgres_types::BorrowToSql;
use xitca_io::bytes::BytesMut;

use super::{
    client::Client, column::Column, driver::Response, error::Error, iter::slice_iter, iter::AsyncIterator,
    statement::Statement, Row, ToSql,
};

/// A pipelined sql query type. It lazily batch queries into local buffer and try to send it
/// with the least amount of syscall when pipeline starts.
///
/// # Examples
/// ```rust
/// use xitca_postgres::{AsyncIterator, Client};
/// async fn pipeline(client: &Client) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
///     let statement = client.prepare("SELECT * FROM public.users", &[]).await?;
///
///     let mut pipe = client.pipeline();
///     pipe.query(statement.as_ref(), &[])?;
///     pipe.query_raw::<[i32; 0]>(statement.as_ref(), [])?;
///
///     let mut res = pipe.run().await?;
///
///     while let Some(item) = res.next().await {
///         let mut item = item?;
///         while let Some(row) = item.next().await {
///             let row = row?;
///             let _: u32 = row.get("id");
///         }
///     }
///
///     Ok(())
/// }
/// ```
pub struct Pipeline<'a, const SYNC_MODE: bool = true> {
    client: &'a Client,
    columns: VecDeque<&'a [Column]>,
    // how many SYNC message we are sending to database.
    // it determines when the driver would shutdown the pipeline.
    sync_count: usize,
    buf: BytesMut,
}

impl<'a, const SYNC_MODE: bool> Pipeline<'a, SYNC_MODE> {
    fn new(client: &'a Client) -> Self {
        Self {
            client,
            columns: VecDeque::new(),
            sync_count: 0,
            buf: BytesMut::new(),
        }
    }
}

impl Client {
    /// start a new pipeline.
    ///
    /// pipeline is sync by default. which means every query inside is considered separate binding
    /// and the pipeline is transparent to database server. the pipeline only happen on socket
    /// transport where minimal amount of syscall is needed.
    ///
    /// for more relaxed [Pipeline Mode][libpq_link] see [Pipeline::pipeline_unsync] api.
    ///
    /// [libpq_link]: https://www.postgresql.org/docs/current/libpq-pipeline-mode.html
    #[inline]
    pub fn pipeline(&self) -> Pipeline<'_> {
        Pipeline::new(self)
    }

    /// start a new un-sync pipeline.
    ///
    /// in un-sync mode pipeline treat all queries inside as one single binding and database server
    /// can see them as no sync point in between which can result in potential performance gain.
    ///
    /// it behaves the same on transportation level as [Pipeline::pipeline_unsync] where minimal
    /// amount of socket syscall is needed.
    #[inline]
    pub fn pipeline_unsync(&self) -> Pipeline<'_, false> {
        Pipeline::new(self)
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

    /// execute the pipeline.
    pub async fn run(mut self) -> Result<PipelineStream<'a>, Error> {
        if self.buf.is_empty() {
            todo!("add error for empty pipeline");
        }

        if !SYNC_MODE {
            self.sync_count += 1;
            frontend::sync(&mut self.buf);
        }

        let res = self.client.tx.send_multi(self.sync_count, self.buf).await?;

        Ok(PipelineStream {
            res,
            columns: self.columns,
            ranges: Vec::new(),
        })
    }
}

/// streaming response of pipeline.
/// impls [AsyncIterator] trait and can be collected asynchronously.
pub struct PipelineStream<'a> {
    res: Response,
    columns: VecDeque<&'a [Column]>,
    ranges: Vec<Option<Range<usize>>>,
}

impl<'a> AsyncIterator for PipelineStream<'a> {
    type Future<'f> = impl Future<Output = Option<Self::Item<'f>>> + 'f where Self: 'f;
    type Item<'i> = Result<PipelineItem<'i, 'a>, Error> where Self: 'i;

    fn next(&mut self) -> Self::Future<'_> {
        async {
            while !self.columns.is_empty() {
                match self.res.recv().await {
                    Ok(msg) => match msg {
                        backend::Message::BindComplete => {
                            return Some(Ok(PipelineItem {
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
                        _ => return Some(Err(Error::UnexpectedMessage)),
                    },
                    Err(e) => return Some(Err(e)),
                }
            }

            None
        }
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

impl Drop for PipelineItem<'_, '_> {
    fn drop(&mut self) {
        self.stream.columns.pop_front();
    }
}

impl AsyncIterator for PipelineItem<'_, '_> {
    type Future<'f> = impl Future<Output=Option<Self::Item<'f>>> + 'f where Self: 'f;
    type Item<'i> = Result<Row<'i>, Error> where Self: 'i;

    fn next(&mut self) -> Self::Future<'_> {
        async {
            while !self.finished {
                match self.stream.res.recv().await {
                    Ok(msg) => match msg {
                        backend::Message::DataRow(body) => {
                            return Some(Row::try_new(
                                self.stream.columns.front().unwrap(),
                                body,
                                &mut self.stream.ranges,
                            ))
                        }
                        backend::Message::CommandComplete(body) => {
                            self.finished = true;
                            self.rows_affected = match crate::query::decode::body_to_affected_rows(&body) {
                                Ok(rows) => rows,
                                Err(e) => return Some(Err(e)),
                            };
                        }
                        _ => return Some(Err(Error::UnexpectedMessage)),
                    },
                    Err(e) => return Some(Err(e)),
                }
            }

            None
        }
    }
}
