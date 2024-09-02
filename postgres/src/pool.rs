use core::{
    future::{Future, IntoFuture},
    ops::DerefMut,
    sync::atomic::Ordering,
};

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use tokio::sync::{Notify, RwLock, RwLockReadGuard, Semaphore, SemaphorePermit};
use xitca_io::bytes::BytesMut;
use xitca_service::pipeline::PipelineE;

use crate::error::DriverDown;

use super::{
    client::Client,
    config::Config,
    driver::connect,
    error::Error,
    iter::slice_iter,
    pipeline::{Owned, Pipeline, PipelineStream},
    statement::{Statement, StatementGuarded},
    BorrowToSql, RowSimpleStream, RowStream, ToSql, Type,
};

pub struct ExclusivePool {
    conn: Mutex<VecDeque<ExclusiveClient>>,
    permits: Semaphore,
}

struct ExclusiveClient {
    client: Client,
    statements: HashMap<Box<str>, Arc<Statement>>,
}

impl ExclusivePool {
    pub async fn get(&self) -> Result<ExclusiveConnection<'_>, Error> {
        let _permit = self.permits.acquire().await.expect("Semaphore must not be closed");
        match self.conn.lock().unwrap().pop_front() {
            None => todo!(),
            Some(conn) => Ok(ExclusiveConnection {
                pool: self,
                conn: Some((conn, _permit)),
            }),
        }
    }
}

pub struct ExclusiveConnection<'a> {
    pool: &'a ExclusivePool,
    conn: Option<(ExclusiveClient, SemaphorePermit<'a>)>,
}

impl<'p> ExclusiveConnection<'p> {
    pub async fn prepare(&mut self, query: &str, types: &[Type]) -> Result<Arc<Statement>, Error> {
        let conn = self.try_conn()?;
        match conn.statements.get(query) {
            Some(stmt) => Ok(stmt.clone()),
            None => {
                let stmt = conn.client.prepare(query, types).await?.leak();
                let stmt = Arc::new(stmt);
                conn.statements.insert(Box::from(query), stmt.clone());
                Ok(stmt)
            }
        }
    }

    pub fn query<'a, I>(&mut self, stmt: &'a Statement, params: I) -> Result<RowStream<'a>, Error>
    where
        I: IntoIterator + Clone,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        self.try_conn()?.client.query_raw(stmt, params).inspect_err(|e| {
            if e.is_driver_down() {
                let _ = self.leak();
            }
        })
    }

    pub fn pipeline<'a, B, const SYNC_MODE: bool>(
        &mut self,
        pipe: Pipeline<'a, B, SYNC_MODE>,
    ) -> Result<PipelineStream<'a>, Error>
    where
        B: DerefMut<Target = BytesMut>,
    {
        self.try_conn()?.client.pipeline(pipe).inspect_err(|e| {
            if e.is_driver_down() {
                let _ = self.leak();
            }
        })
    }

    fn try_conn(&mut self) -> Result<&mut ExclusiveClient, Error> {
        match self.conn {
            Some((ref mut conn, _)) => Ok(conn),
            None => Err(DriverDown.into()),
        }
    }

    fn leak(&mut self) -> (ExclusiveClient, SemaphorePermit<'p>) {
        self.conn
            .take()
            .expect("connection must not be leaked when it's already broken")
    }
}

impl Drop for ExclusiveConnection<'_> {
    fn drop(&mut self) {
        if let Some((conn, _permit)) = self.conn.take() {
            self.pool.conn.lock().unwrap().push_back(conn);
        }
    }
}

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

impl Persist {
    fn spawn_guard(&self) -> SpawnGuard<'_> {
        SpawnGuard(self)
    }
}

struct Spawner {
    notify: Mutex<Option<Arc<Notify>>>,
}

impl Spawner {
    #[cold]
    #[inline(never)]
    fn spawn_or_wait(&self) -> Option<Arc<Notify>> {
        let mut lock = self.notify.lock().unwrap();
        match *lock {
            Some(ref notify) => Some(notify.clone()),
            None => {
                *lock = Some(Arc::new(Notify::new()));
                None
            }
        }
    }

    #[cold]
    #[inline(never)]
    async fn wait_for_spawn(&self) {
        let notify = self.notify.lock().unwrap().clone();
        if let Some(notify) = notify {
            notify.notified().await;
        }
    }
}

struct SpawnGuard<'a>(&'a Persist);

impl Drop for SpawnGuard<'_> {
    fn drop(&mut self) {
        // if for any reason current task is cancelled by user the drop guard would
        // restore the spawning state.
        if let Some(notify) = self.0.spawner.notify.lock().unwrap().take() {
            notify.notify_waiters();
        }
    }
}

impl SpawnGuard<'_> {
    #[cold]
    #[inline(never)]
    async fn spawn(&self) -> Client {
        let (cli, drv) = {
            loop {
                match connect(&mut self.0.config.clone()).await {
                    Ok(res) => break res,
                    Err(_) => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
                }
            }
        };

        tokio::task::spawn(drv.into_future());

        for (id, query, types) in self.0.statements_cache.iter() {
            let _ = cli.prepare_with_id(*id, query.as_str(), types.as_slice()).await;
        }

        cli
    }
}

impl SharedClient {
    pub async fn new<C>(config: C) -> Result<Self, Error>
    where
        Config: TryFrom<C>,
        Error: From<<Config as TryFrom<C>>::Error>,
    {
        let mut config = Config::try_from(config)?;
        let (cli, drv) = connect(&mut config).await?;

        tokio::task::spawn(drv.into_future());

        Ok(Self {
            inner: RwLock::new(cli),
            persist: Box::new(Persist {
                spawner: Spawner {
                    notify: Mutex::new(None),
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
        I: IntoIterator + Clone,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        let cli = self.read().await;
        match cli.query_raw(stmt, params.clone()) {
            Ok(res) => Ok(res),
            Err(err) => {
                drop(cli);
                Box::pin(self.query_raw_slow(stmt, params, err)).await
            }
        }
    }

    #[cold]
    #[inline(never)]
    async fn query_raw_slow<'a, I>(
        &self,
        stmt: &'a Statement,
        params: I,
        mut err: Error,
    ) -> Result<RowStream<'a>, Error>
    where
        I: IntoIterator + Clone,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        loop {
            if !err.is_driver_down() {
                return Err(err);
            }

            self.reconnect().await;

            match self.read().await.query_raw(stmt, params.clone()) {
                Ok(res) => return Ok(res),
                Err(e) => err = e,
            }
        }
    }

    pub async fn query_simple(&self, stmt: &str) -> Result<RowSimpleStream, Error> {
        let cli = self.read().await;
        match cli.query_simple(stmt) {
            Ok(res) => Ok(res),
            Err(e) => {
                drop(cli);
                Box::pin(self.query_simple_slow(stmt, e)).await
            }
        }
    }

    #[cold]
    #[inline(never)]
    async fn query_simple_slow(&self, stmt: &str, mut err: Error) -> Result<RowSimpleStream, Error> {
        loop {
            if !err.is_driver_down() {
                return Err(err);
            }

            self.reconnect().await;

            match self.read().await.query_simple(stmt) {
                Ok(res) => return Ok(res),
                Err(e) => err = e,
            }
        }
    }

    pub async fn prepare(
        &self,
        query: &str,
        types: &[Type],
    ) -> Result<StatementGuarded<RwLockReadGuard<'_, Client>>, Error> {
        loop {
            let cli = self.read().await;
            match cli._prepare(query, types).await {
                Ok(stmt) => return Ok(stmt.into_guarded(cli)),
                Err(e) => {
                    if !e.is_driver_down() {
                        return Err(e);
                    }
                    drop(cli);
                    Box::pin(self.reconnect()).await;
                }
            }
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

    #[cold]
    #[inline(never)]
    async fn reconnect(&self) {
        match self.persist.spawner.spawn_or_wait() {
            Some(wait) => wait.notified().await,
            None => {
                let guard = self.persist.spawn_guard();

                let mut cli = self.inner.write().await;

                *cli = guard.spawn().await;

                // release rwlock before spawn guard. when waiters are notified it's important that the lock
                // is free for read lock.
                drop(cli);
                drop(guard);
            }
        }
    }

    async fn read(&self) -> RwLockReadGuard<'_, Client> {
        loop {
            match self.inner.try_read() {
                Ok(cli) => return cli,
                // failing to acquire read lock means certain task is spawning new connection.
                // if there is no notify existing in spawner it means the spawn process has finished(or cancelled).
                // in that case just try read lock again.
                Err(_) => self.persist.spawner.wait_for_spawn().await,
            }
        }
    }
}

impl SharedClient {
    pub fn pipeline<'a, 's, 'f, B, const SYNC_MODE: bool>(
        &'s self,
        pipe: Pipeline<'a, B, SYNC_MODE>,
    ) -> impl Future<Output = Result<PipelineStream<'a>, Error>> + Send + 'f
    where
        'a: 'f,
        's: 'f,
        B: DerefMut<Target = BytesMut> + Into<Owned>,
    {
        let opt = match self.inner.try_read() {
            Ok(cli) => PipelineE::First(cli.pipeline(pipe)),
            Err(_) => PipelineE::Second(pipe.into_owned()),
        };

        async {
            match opt {
                PipelineE::First(res) => res,
                PipelineE::Second(pipe) => Box::pin(self.pipeline_slow(pipe)).await,
            }
        }
    }

    #[cold]
    #[inline(never)]
    async fn pipeline_slow<'a, const SYNC_MODE: bool>(
        &self,
        mut pipe: Pipeline<'a, Owned, SYNC_MODE>,
    ) -> Result<PipelineStream<'a>, Error> {
        let Pipeline { columns, ref mut buf } = pipe;
        let cli = self.read().await;
        match cli._pipeline::<SYNC_MODE, true>(&columns, buf) {
            Ok(res) => Ok(PipelineStream::new(res, columns)),
            Err(mut err) => {
                drop(cli);
                loop {
                    if !err.is_driver_down() {
                        return Err(err);
                    }

                    self.reconnect().await;

                    match self.read().await._pipeline::<SYNC_MODE, false>(&columns, buf) {
                        Ok(res) => return Ok(PipelineStream::new(res, columns)),
                        Err(e) => err = e,
                    }
                }
            }
        }
    }
}
