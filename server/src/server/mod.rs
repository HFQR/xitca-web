mod future;
mod handle;
mod service;

pub use self::{future::ServerFuture, handle::ServerHandle};

pub(crate) use self::service::{IntoServiceObj, ServiceObj};

use std::{
    io, mem,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use tokio::{
    runtime::Runtime,
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
};
use tokio_util::sync::CancellationToken;

use crate::{builder::Builder, worker};

pub struct Server {
    is_graceful_shutdown: Arc<AtomicBool>,
    tx_cmd: UnboundedSender<Command>,
    rx_cmd: UnboundedReceiver<Command>,
    rt: Option<Runtime>,
    cancellation_token: CancellationToken,
    worker_join_handles: Vec<thread::JoinHandle<io::Result<()>>>,
}

impl Server {
    #[cfg(target_family = "wasm")]
    pub fn new(builder: Builder) -> io::Result<Self> {
        let Builder {
            listeners,
            factories,
            shutdown_timeout,
            on_worker_start,
            ..
        } = builder;

        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;

        let fut = async {
            listeners
                .into_iter()
                .flat_map(|(name, listeners)| listeners.into_iter().map(move |l| l().map(|l| (name.to_owned(), l))))
                .collect::<Result<Vec<_>, io::Error>>()
        };

        let listeners = rt.block_on(fut)?;

        let is_graceful_shutdown = Arc::new(AtomicBool::new(false));

        let on_start_fut = on_worker_start();

        let fut = async {
            on_start_fut.await;

            let mut handles = Vec::new();
            let mut services = Vec::new();

            for (name, factory) in factories.iter() {
                let (h, s) = factory
                    .call((name, &listeners))
                    .await
                    .map_err(|_| io::Error::from(io::ErrorKind::Other))?;
                handles.extend(h);
                services.push(s);
            }

            worker::wait_for_stop(handles, services, shutdown_timeout, &is_graceful_shutdown).await;

            Ok::<_, io::Error>(())
        };

        rt.block_on(tokio::task::LocalSet::new().run_until(fut))?;

        unreachable!("")
    }

    #[cfg(not(target_family = "wasm"))]
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
                .flat_map(|(name, listeners)| listeners.into_iter().map(move |l| l().map(|l| (name.to_owned(), l))))
                .collect::<Result<Vec<_>, io::Error>>()
        };

        // use a spawned thread to work around possible nest runtime issue.
        // *. Server::new is most likely already inside a tokio runtime.
        let listeners = thread::scope(|s| s.spawn(|| rt.block_on(fut)).join()).unwrap()?;

        let is_graceful_shutdown = Arc::new(AtomicBool::new(false));
        let is_graceful_shutdown2 = is_graceful_shutdown.clone();
        let cancellation_token = CancellationToken::new();
        let cancellation_token2 = cancellation_token.clone();

        let worker_handles = thread::Builder::new()
            .name(String::from("xitca-server-worker-shared-scope"))
            .spawn(move || {
                let is_graceful_shutdown = is_graceful_shutdown2;

                // TODO: wait for startup error(including panic) and return as io::Error on call site.
                // currently the error only show when shared scope thread is joined with handle.
                thread::scope(|scope| {
                    for idx in 0..worker_threads {
                        let thread = thread::Builder::new().name(format!("xitca-server-worker-{idx}"));

                        let task = || async {
                            on_worker_start().await;

                            let mut handles = Vec::new();
                            let mut services = Vec::new();

                            for (name, factory) in factories.iter() {
                                match factory.call((name, &listeners, cancellation_token2.clone())).await {
                                    Ok((h, s)) => {
                                        handles.extend(h);
                                        services.push(s);
                                    }
                                    Err(_) => return,
                                }
                            }

                            worker::wait_for_stop(handles, services, shutdown_timeout, &is_graceful_shutdown).await;
                        };

                        #[cfg(not(feature = "io-uring"))]
                        {
                            let rt = tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .max_blocking_threads(worker_max_blocking_threads)
                                .build()?;

                            thread.spawn_scoped(scope, move || {
                                rt.block_on(tokio::task::LocalSet::new().run_until(task()))
                            })?;
                        }

                        #[cfg(feature = "io-uring")]
                        {
                            thread.spawn_scoped(scope, move || {
                                let _ = worker_max_blocking_threads;
                                tokio_uring_xitca::start(task())
                            })?;
                        }
                    }

                    Ok(())
                })
            })?;

        let (tx_cmd, rx_cmd) = tokio::sync::mpsc::unbounded_channel();

        Ok(Self {
            is_graceful_shutdown,
            tx_cmd,
            rx_cmd,
            rt: Some(rt),
            worker_join_handles: vec![worker_handles],
            cancellation_token,
        })
    }

    pub(crate) fn stop(&mut self, graceful: bool) {
        if let Some(rt) = self.rt.take() {
            self.is_graceful_shutdown.store(graceful, Ordering::SeqCst);
            self.cancellation_token.cancel();

            rt.shutdown_background();
            mem::take(&mut self.worker_join_handles).into_iter().for_each(|handle| {
                let _ = handle.join().unwrap();
            });
        }
    }
}

enum Command {
    GracefulStop,
    ForceStop,
}
