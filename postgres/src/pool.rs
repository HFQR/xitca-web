use core::{future::IntoFuture, sync::atomic::Ordering};

use std::sync::Arc;

use postgres_types::{BorrowToSql, ToSql, Type};
use tokio::sync::{Notify, RwLock, RwLockReadGuard};

use crate::{
    client::Client,
    config::Config,
    driver::connect,
    error::Error,
    iter::slice_iter,
    statement::{Statement, StatementGuarded},
    util::lock::Lock,
    RowSimpleStream, RowStream,
};

/// a shared connection for non transaction queries and [Statement] cache live as long as the connection itself.
pub struct SharedClient {
    inner: RwLock<Client>,
    persist: Box<Persist>,
}

struct Persist {
    spawner: Spawner,
    config: Config,
    statements_cache: Vec<(usize, String, Vec<Type>)>,
}

struct Spawner {
    notify: Lock<Option<Arc<Notify>>>,
}

impl Spawner {
    #[cold]
    #[inline(never)]
    fn spawn_or_wait(&self) -> Option<Arc<Notify>> {
        let mut lock = self.notify.lock();
        match *lock {
            Some(ref notify) => Some(notify.clone()),
            None => {
                *lock = Some(Arc::new(Notify::new()));
                None
            }
        }
    }
}

impl SharedClient {
    pub async fn new(config: Config) -> Result<Self, Error> {
        let (cli, drv) = connect(&mut config.clone()).await?;

        tokio::task::spawn(drv.into_future());

        Ok(Self {
            inner: RwLock::new(cli),
            persist: Box::new(Persist {
                spawner: Spawner {
                    notify: Lock::new(None),
                },
                config,
                statements_cache: Vec::new(),
            }),
        })
    }

    #[inline]
    pub async fn query<'a>(&self, stmt: &'a Statement, params: &[&(dyn ToSql + Sync)]) -> Result<RowStream<'a>, Error> {
        self.query_raw(stmt, slice_iter(params)).await
    }

    pub async fn query_raw<'a, I>(&self, stmt: &'a Statement, params: I) -> Result<RowStream<'a>, Error>
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        let cli = self.inner.read().await;
        match cli.query_raw(stmt, params).await {
            Err(Error::DriverDown(msg)) => {
                drop(cli);
                Box::pin(async move {
                    self.reconnect().await;
                    self.inner.read().await.query_buf(stmt, msg).await
                })
                .await
            }
            res => res,
        }
    }

    pub async fn query_simple(&self, stmt: &str) -> Result<RowSimpleStream, Error> {
        let cli = self.inner.read().await;
        match cli.query_simple(stmt).await {
            Err(Error::DriverDown(msg)) => {
                drop(cli);
                Box::pin(async move {
                    self.reconnect().await;
                    self.inner.read().await.query_buf_simple(msg).await
                })
                .await
            }
            res => res,
        }
    }

    pub async fn prepare(
        &self,
        query: &str,
        types: &[Type],
    ) -> Result<StatementGuarded<RwLockReadGuard<'_, Client>>, Error> {
        let cli = self.inner.read().await;
        match cli._prepare(query, types).await {
            Ok(stmt) => Ok(stmt.into_guarded(cli)),
            Err(Error::DriverDown(_)) => {
                drop(cli);
                Box::pin(async move {
                    self.reconnect().await;
                    self.prepare(query, types).await
                })
                .await
            }
            Err(e) => Err(e),
        }
    }

    /// cached statement that would live as long as SharedClient.
    pub async fn prepare_cached(&mut self, query: &str, types: &[Type]) -> Result<Statement, Error> {
        let cli = self.inner.read().await;
        let id = crate::prepare::NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let stmt = cli.prepare_with_id(id, query, types).await?;

        self.persist
            .statements_cache
            .push((id, String::from(query), types.into()));

        Ok(stmt)
    }

    #[cfg(not(feature = "quic"))]
    pub async fn pipeline<'a, const SYNC_MODE: bool>(
        &self,
        mut pipe: crate::pipeline::Pipeline<'a, SYNC_MODE>,
    ) -> Result<crate::pipeline::PipelineStream<'a>, Error> {
        let cli = self.inner.read().await;
        match cli._pipeline::<SYNC_MODE>(&mut pipe.sync_count, pipe.buf).await {
            Ok(res) => Ok(crate::pipeline::PipelineStream {
                res,
                columns: pipe.columns,
                ranges: Vec::new(),
            }),
            Err(Error::DriverDown(buf)) => {
                drop(cli);
                Box::pin(async move {
                    self.reconnect().await;
                    self.inner
                        .read()
                        .await
                        .pipeline_buf(pipe.sync_count, buf, pipe.columns)
                        .await
                })
                .await
            }
            Err(e) => Err(e),
        }
    }

    #[cold]
    #[inline(never)]
    async fn reconnect(&self) {
        match self.persist.spawner.spawn_or_wait() {
            Some(wait) => wait.notified().await,
            None => {
                // if for any reason current task is cancelled by user the drop guard would
                // restore the spawning state.
                struct SpawnGuard<'a>(&'a Spawner);

                impl Drop for SpawnGuard<'_> {
                    fn drop(&mut self) {
                        if let Some(notify) = self.0.notify.lock().take() {
                            notify.notify_waiters();
                        }
                    }
                }

                let mut cli = self.inner.write().await;

                let _guard = SpawnGuard(&self.persist.spawner);

                let (cli_new, drv) = {
                    loop {
                        match connect(&mut self.persist.config.clone()).await {
                            Ok(res) => break res,
                            Err(_) => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
                        }
                    }
                };

                tokio::task::spawn(drv.into_future());

                for (id, query, types) in self.persist.statements_cache.iter() {
                    let _ = cli_new.prepare_with_id(*id, query.as_str(), types.as_slice()).await;
                }

                *cli = cli_new;
            }
        }
    }
}
