mod future;
mod handle;
mod service;

pub use self::future::{ServerFuture, ServerFutureInner};
pub use self::handle::ServerHandle;

pub(crate) use self::service::{AsServiceFactoryClone, Factory, ServiceFactoryClone};

use std::{
    io, mem,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
};

use log::{error, info};
use tokio::{
    runtime,
    sync::{
        mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
        oneshot,
    },
    task::LocalSet,
};

use crate::builder::Builder;

pub struct Server {
    is_graceful_shutdown: Arc<AtomicBool>,
    tx_cmd: UnboundedSender<Command>,
    rx_cmd: UnboundedReceiver<Command>,
    server_join_handle: Option<thread::JoinHandle<()>>,
    tx: Option<oneshot::Sender<()>>,
    worker_join_handles: Vec<thread::JoinHandle<()>>,
}

impl Server {
    pub fn new(builder: Builder) -> io::Result<Self> {
        let Builder {
            server_threads,
            worker_threads,
            worker_max_blocking_threads,
            connection_limit,
            listeners,
            factories,
            shutdown_timeout,
            ..
        } = builder;

        let (tx, rx) = std::sync::mpsc::sync_channel(1);

        let server_handle = thread::spawn(move || {
            let res = runtime::Builder::new_multi_thread()
                .enable_all()
                // This worker threads is only used for accepting connections.
                // actix-sever worker does not run task on them.
                .worker_threads(server_threads)
                .build()
                .and_then(|rt| {
                    let res = rt.block_on(async {
                        listeners
                            .into_iter()
                            .map(|(name, listeners)| listeners.into_iter().map(move |l| (name.to_owned(), l)))
                            .flatten()
                            .map(|(name, mut l)| {
                                let l = l.as_listener()?;
                                Ok((name, Arc::new(l)))
                            })
                            .collect::<Result<Vec<_>, io::Error>>()
                    })?;

                    Ok((res, rt))
                });

            let (tx2, rx2) = oneshot::channel();

            match res {
                Ok((listeners, rt)) => {
                    tx.send((tx2, Ok(listeners))).unwrap();

                    rt.block_on(async {
                        if rx2.await.is_err() {
                            error!("Force stopped Accept. ServerFuture dropped unexpectedly.");
                        } else {
                            info!("Graceful stopped Accept.");
                        };
                    })
                }
                Err(e) => {
                    tx.send((tx2, Err(e))).unwrap();
                }
            }
        });

        let (tx, res) = rx.recv().unwrap();
        let listeners = res?;

        let is_graceful_shutdown = Arc::new(AtomicBool::new(false));

        let worker_handles = (0..worker_threads)
            .map(|idx| {
                let is_graceful_shutdown = is_graceful_shutdown.clone();
                let listeners = listeners.clone();
                let factories = factories
                    .iter()
                    .map(|(name, factory)| (name.to_owned(), factory.clone_factory()))
                    .collect::<Vec<_>>();

                let (tx, rx) = std::sync::mpsc::sync_channel(1);

                let handle = thread::Builder::new()
                    .name(format!("actix-server-worker-{}", idx))
                    .spawn(move || {
                        let rt = runtime::Builder::new_current_thread()
                            .enable_all()
                            .max_blocking_threads(worker_max_blocking_threads)
                            .build()
                            .unwrap();
                        let local = LocalSet::new();

                        let services = rt.block_on(local.run_until(async {
                            let mut services = Vec::new();

                            for (name, factory) in factories {
                                let service = factory.new_service().await?;
                                services.push((name, service));
                            }

                            Ok::<_, ()>(services)
                        }));

                        match services {
                            Ok(services) => {
                                tx.send(Ok(())).unwrap();

                                rt.block_on(local.run_until(async {
                                    crate::worker::run(
                                        listeners,
                                        services,
                                        connection_limit,
                                        shutdown_timeout,
                                        is_graceful_shutdown,
                                    )
                                    .await;
                                }))
                            }
                            Err(_) => {
                                tx.send(Err(io::Error::new(
                                    io::ErrorKind::Other,
                                    "Worker Services fail to start",
                                )))
                                .unwrap();
                            }
                        }
                    })?;

                rx.recv().unwrap()?;

                Ok(handle)
            })
            .collect::<io::Result<Vec<_>>>()?;

        let (tx_cmd, rx_cmd) = unbounded_channel();

        Ok(Self {
            is_graceful_shutdown,
            tx_cmd,
            rx_cmd,
            server_join_handle: Some(server_handle),
            tx: Some(tx),
            worker_join_handles: worker_handles,
        })
    }

    pub(crate) fn stop(&mut self, graceful: bool) {
        self.is_graceful_shutdown.store(graceful, Ordering::SeqCst);

        self.tx
            .take()
            .unwrap()
            .send(())
            .expect("Accept thread exited unexpectedly");

        self.server_join_handle.take().unwrap().join().unwrap();

        mem::take(&mut self.worker_join_handles).into_iter().for_each(|handle| {
            handle.join().unwrap();
        });
    }
}

enum Command {
    GracefulStop,
    ForceStop,
}
