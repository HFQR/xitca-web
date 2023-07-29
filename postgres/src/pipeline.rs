use core::{future::Future, mem, ops::Range};

use std::collections::VecDeque;

use postgres_protocol::message::{backend, frontend};
use postgres_types::BorrowToSql;
use xitca_io::bytes::BytesMut;

use super::{
    client::Client, column::Column, driver::Response, error::Error, iter::AsyncIterator, statement::Statement, Row,
};

/// A pipelined sql query type. It lazily batch queries into local buffer and try to send it
/// with the least amount of syscall when pipeline starts.
///
/// by default pipeline adds SYNC message to every query inside the pipeline.
/// see [Pipeline::un_sync] for detail if un-sync mode is desired.
pub struct Pipeline<'a, const SYNC_MODE: bool = true> {
    client: &'a Client,
    columns: VecDeque<&'a [Column]>,
    buf: BytesMut,
}

impl Client {
    /// start a new pipeline.
    #[inline]
    pub fn pipeline(&self) -> Pipeline<'_> {
        Pipeline {
            client: self,
            columns: VecDeque::new(),
            buf: BytesMut::new(),
        }
    }
}

impl<'a> Pipeline<'a> {
    /// switch Pipeline to un-sync mode.
    /// after go into un-sync mode new query added to pipeline is not attached with SYNC message.
    /// (queries already inside the pipeline is not affected by mode change)
    /// instead a single SYNC message is attached to pipeline when [Pipeline::run] method is called.
    #[inline]
    pub fn unsync(self) -> Pipeline<'a, false> {
        Pipeline {
            client: self.client,
            columns: self.columns,
            buf: self.buf,
        }
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
            })
            .map_err(|e| {
                // revert back to last pipelined query when encoding error occurred.
                self.buf.truncate(len);
                e
            })
    }

    /// execute the pipeline and send previous batched queries to server.
    pub async fn run(mut self) -> Result<PipelineStream<'a>, Error> {
        if !SYNC_MODE {
            frontend::sync(&mut self.buf);
        }

        let count = self.columns.len();
        let res = self.client.tx.send_multi(count, self.buf).await?;

        Ok(PipelineStream {
            res,
            columns: self.columns,
            ranges: Vec::new(),
            consume_last_msg: false,
            next_rows_affected: 0,
        })
    }
}

pub struct PipelineStream<'a> {
    res: Response,
    columns: VecDeque<&'a [Column]>,
    ranges: Vec<Option<Range<usize>>>,
    consume_last_msg: bool,
    next_rows_affected: u64,
}

impl<'a> AsyncIterator for PipelineStream<'a> {
    type Future<'f> = impl Future<Output = Option<Self::Item<'f>>> + 'f where Self: 'f;
    type Item<'i> = Result<PipelineItem<'i, 'a>, Error> where Self: 'i;

    fn next(&mut self) -> Self::Future<'_> {
        async {
            if self.columns.is_empty() {
                return None;
            }

            // if previous PipelineItem is dropped mid flight do some catch up.
            while self.consume_last_msg {
                match self.res.recv().await {
                    Ok(msg) => match msg {
                        backend::Message::DataRow(_) => {}
                        backend::Message::ReadyForQuery(_) => {
                            self.consume_last_msg = false;
                        }
                        _ => return Some(Err(Error::UnexpectedMessage)),
                    },
                    Err(e) => return Some(Err(e)),
                }
            }

            loop {
                match self.res.recv().await {
                    Ok(msg) => match msg {
                        backend::Message::DataRow(body) => {
                            self.consume_last_msg = true;
                            let rows_affected = mem::replace(&mut self.next_rows_affected, 0);
                            return Some(Ok(PipelineItem {
                                finished: false,
                                stream: self,
                                first_row: Some(body),
                                rows_affected,
                            }));
                        }
                        backend::Message::EmptyQueryResponse => self.next_rows_affected = 0,
                        // TODO: parse command complete body to rows affected.
                        backend::Message::CommandComplete(body) => {
                            self.next_rows_affected = match crate::query::decode::body_to_affected_rows(&body) {
                                Ok(rows) => rows,
                                Err(e) => return Some(Err(e)),
                            }
                        }
                        backend::Message::PortalSuspended => {}
                        backend::Message::ReadyForQuery(_) => {
                            let rows_affected = mem::replace(&mut self.next_rows_affected, 0);
                            return Some(Ok(PipelineItem {
                                finished: true,
                                stream: self,
                                first_row: None,
                                rows_affected,
                            }));
                        }
                        _ => return Some(Err(Error::UnexpectedMessage)),
                    },
                    Err(e) => return Some(Err(e)),
                }
            }
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
    first_row: Option<backend::DataRowBody>,
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
            if self.finished {
                return None;
            }

            if let Some(first) = self.first_row.take() {
                return Some(Row::try_new(
                    self.stream.columns.front().unwrap(),
                    first,
                    &mut self.stream.ranges,
                ));
            }

            match self.stream.res.recv().await {
                Ok(msg) => match msg {
                    backend::Message::DataRow(body) => Some(Row::try_new(
                        self.stream.columns.front().unwrap(),
                        body,
                        &mut self.stream.ranges,
                    )),
                    backend::Message::ReadyForQuery(_) => {
                        self.stream.consume_last_msg = false;
                        self.finished = true;
                        None
                    }
                    _ => Some(Err(Error::UnexpectedMessage)),
                },
                Err(e) => Some(Err(e)),
            }
        }
    }
}
