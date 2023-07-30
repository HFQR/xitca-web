use core::{future::Future, ops::Range};

use std::collections::VecDeque;

use postgres_protocol::message::{backend, frontend};
use postgres_types::BorrowToSql;
use xitca_io::bytes::BytesMut;

use super::{
    client::Client, column::Column, driver::Response, error::Error, iter::AsyncIterator, statement::Statement, Row,
};

/// A pipelined sql query type. It lazily batch queries into local buffer and try to send it
/// with the least amount of syscall when pipeline starts.
pub struct Pipeline<'a, const SYNC_MODE: bool = true> {
    client: &'a Client,
    columns: VecDeque<&'a [Column]>,
    // how many SNYC message we are sending to database.
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
    #[inline]
    pub fn pipeline(&self) -> Pipeline<'_> {
        Pipeline::new(self)
    }

    /// start a new un-sync pipeline.
    /// in un-sync mode query added to pipeline is not attached with SYNC message.
    /// instead a single SYNC message is attached to pipeline when [Pipeline::run] method is called.
    #[inline]
    pub fn pipeline_unsync(&self) -> Pipeline<'_, false> {
        Pipeline::new(self)
    }
}

impl<'a, const SYNC_MODE: bool> Pipeline<'a, SYNC_MODE> {
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

    /// execute the pipeline and send previous batched queries to server.
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
                            // last PieplineItem dropped before finish. do some catch up until next
                            // next item arrives.
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

pub struct PipelineItem<'a, 'c> {
    finished: bool,
    stream: &'a mut PipelineStream<'c>,
    rows_affected: u64,
}

impl PipelineItem<'_, '_> {
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
