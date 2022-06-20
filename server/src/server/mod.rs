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

use tokio::{
    runtime::Runtime,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
};

use crate::{builder::Builder, worker};

pub struct Server {
    is_graceful_shutdown: Arc<AtomicBool>,
    tx_cmd: UnboundedSender<Command>,
    rx_cmd: UnboundedReceiver<Command>,
    rt: Option<Runtime>,
    worker_join_handles: Vec<thread::JoinHandle<()>>,
}

impl Server {
    pub fn new(builder: Builder) -> io::Result<Self> {
        let Builder {
            server_threads,
            worker_threads,
            worker_max_blocking_threads,
            listeners,
            factories,
            shutdown_timeout,
            on_worker_start,
            ..
        } = builder;

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            // This worker threads is only used for accepting connections.
            // xitca-sever worker does not run task on them.
            .worker_threads(server_threads)
            .build()?;

        let fut = async {
            listeners
                .into_iter()
                .flat_map(|(name, listeners)| {
                    listeners.into_iter().map(move |mut l| {
                        let l = l.as_listener()?;
                        Ok((name.to_owned(), Arc::new(l)))
                    })
                })
                .collect::<Result<Vec<_>, io::Error>>()
        };

        // use a spawned thread to work around possible nest runtime issue.
        // *. Server::new is most likely already inside a tokio runtime.
        let listeners = thread::scope(|s| s.spawn(|| rt.block_on(fut)).join())
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))??;

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

                let on_start_fut = on_worker_start();

                let handle = thread::Builder::new()
                    .name(format!("xitca-server-worker-{}", idx))
                    .spawn(move || {
                        let fut = async {
                            on_start_fut.await;

                            // TODO: max blocking thread configure is needed.
                            let _ = worker_max_blocking_threads;

                            // async move is IMPORTANT here.
                            // `Listener` may do extra work with `Drop` trait and in order to prevent `Arc<Listener>` holding more reference
                            // counts than necessary all extra clone need to be dropped asap.
                            let fut = async move {
                                let mut handles = Vec::new();
                                let mut services = Vec::new();

                                for (name, factory) in factories {
                                    let (h, s) = factory.spawn_handle(&name, &listeners).await?;
                                    handles.extend(h);
                                    services.push(s);
                                }

                                // See above reason for `async move`
                                drop(listeners);

                                Ok::<_, ()>((handles, services))
                            };

                            match fut.await {
                                Ok((handles, services)) => {
                                    tx.send(Ok(())).unwrap();
                                    worker::wait_for_stop(handles, services, shutdown_timeout, is_graceful_shutdown)
                                        .await;
                                }
                                Err(_) => tx
                                    .send(Err(io::Error::new(
                                        io::ErrorKind::Other,
                                        "Worker Services fail to start",
                                    )))
                                    .unwrap(),
                            }
                        };

                        #[cfg(not(feature = "io-uring"))]
                        {
                            tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .max_blocking_threads(worker_max_blocking_threads)
                                .build()
                                .unwrap()
                                .block_on(tokio::task::LocalSet::new().run_until(fut))
                        }
                        #[cfg(feature = "io-uring")]
                        {
                            tokio_uring::start(fut)
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
            rt: Some(rt),
            worker_join_handles: worker_handles,
        })
    }

    pub(crate) fn stop(&mut self, graceful: bool) {
        self.is_graceful_shutdown.store(graceful, Ordering::SeqCst);

        self.rt.take().unwrap().shutdown_background();

        mem::take(&mut self.worker_join_handles).into_iter().for_each(|handle| {
            handle.join().unwrap();
        });
    }
}

enum Command {
    GracefulStop,
    ForceStop,
}
