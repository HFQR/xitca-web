mod future;
mod handle;
mod service;

pub use handle::ServerHandle;

pub(crate) use self::future::{ServerFuture, ServerFutureInner};
pub(crate) use self::service::{Factory, IntoServiceFactoryClone, ServiceFactoryClone};

use std::io;
use std::mem;
use std::sync::Arc;
use std::thread;

use log::error;
use tokio::runtime;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio::task::LocalSet;

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
                // Place holder setting and currently no blocking tasks would run
                // in server runtime.
                .max_blocking_threads(max_blocking_threads)
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
                    .collect();

                #[cfg(feature = "actix")]
                let actix = actix_rt::System::try_current();

                thread::spawn(move || {
                    #[cfg(feature = "actix")]
                    if let Some(actix) = actix {
                        actix_rt::System::set_current(actix);
                    }

                    let rt = runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap();
                    let local = LocalSet::new();

                    rt.block_on(local.run_until(async {
                        crate::worker::run(listeners, factories, connection_limit).await;
                    }))
                })
            })
            .collect();

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
