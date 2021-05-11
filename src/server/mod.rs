mod future;
mod handle;
mod service;

pub use self::future::{ServerFuture, ServerFutureInner};
pub use self::handle::ServerHandle;

pub(crate) use self::service::{AsServiceFactoryClone, Factory, ServiceFactoryClone};

use std::{io, mem, sync::Arc, thread};

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
    tx_cmd: UnboundedSender<Command>,
    rx_cmd: UnboundedReceiver<Command>,
    server_join_handle: Option<thread::JoinHandle<()>>,
    tx: Option<oneshot::Sender<()>>,
    worker_join_handles: Vec<thread::JoinHandle<()>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.tx
            .take()
            .unwrap()
            .send(())
            .expect("Accept thread exited unexpectedly");

        self.server_join_handle.take().unwrap().join().unwrap();

        mem::take(&mut self.worker_join_handles)
            .into_iter()
            .for_each(|handle| {
                handle.join().unwrap();
            });
    }
}

impl Server {
    pub fn new(builder: Builder) -> io::Result<Self> {
        let Builder {
            server_threads,
            worker_threads,
            max_blocking_threads,
            connection_limit,
            listeners,
            factories,
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
                            .map(|(name, listeners)| {
                                listeners.into_iter().map(move |l| (name.to_owned(), l))
                            })
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
                            error!("ServerFuture dropped unexpectedly.");
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

        let worker_handles = (0..worker_threads)
            .map(|_| {
                let listeners = listeners.clone();
                let factories = factories
                    .iter()
                    .map(|(name, factory)| (name.to_owned(), factory.clone_factory()))
                    .collect::<Vec<_>>();

                #[cfg(feature = "actix")]
                let actix = actix_rt::System::try_current();

                let (tx, rx) = std::sync::mpsc::sync_channel(1);

                let handle = thread::spawn(move || {
                    #[cfg(feature = "actix")]
                    if let Some(actix) = actix {
                        actix_rt::System::set_current(actix);
                    }

                    let rt = runtime::Builder::new_current_thread()
                        .enable_all()
                        .max_blocking_threads(max_blocking_threads)
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
                                info!("Started worker on {:?}", thread::current().id());
                                crate::worker::run(listeners, services, connection_limit).await;
                                info!("Stopped worker on {:?}", thread::current().id());
                            }))
                        }
                        Err(_) => {
                            tx.send(Err(io::Error::new(
                                io::ErrorKind::Other,
                                "Worker Services failt to start",
                            )))
                            .unwrap();
                        }
                    }
                });

                rx.recv().unwrap()?;

                Ok(handle)
            })
            .collect::<io::Result<Vec<_>>>()?;

        let (tx_cmd, rx_cmd) = unbounded_channel();

        Ok(Self {
            tx_cmd,
            rx_cmd,
            server_join_handle: Some(server_handle),
            tx: Some(tx),
            worker_join_handles: worker_handles,
        })
    }
}

enum Command {
    GracefulStop(oneshot::Sender<()>),
    ForceStop,
}
