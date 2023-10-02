mod future;
mod handle;
mod service;

pub use self::{
    future::{ServerFuture, ServerFutureInner},
    handle::ServerHandle,
};

pub(crate) use self::service::{BuildServiceFn, BuildServiceObj};

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
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
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
                .flat_map(|(name, listeners)| {
                    listeners.into_iter().map(move |mut l| {
                        let l = l.as_listener()?;
                        Ok((name.to_owned(), Arc::new(l)))
                    })
                })
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
            .unwrap_or_else(|e| std::panic::resume_unwind(e))?;

        let is_graceful_shutdown = Arc::new(AtomicBool::new(false));

        let is_graceful_shutdown2 = is_graceful_shutdown.clone();

        let worker_handles = thread::Builder::new()
            .name(String::from("xitca-server-worker-shared-scope"))
            .spawn(move || {
                let is_graceful_shutdown = is_graceful_shutdown2;

                thread::scope(|s| {
                    let mut handles = Vec::with_capacity(worker_threads);

                    let spawner = |scope| {
                        for idx in 0..worker_threads {
                            let thread = thread::Builder::new().name(format!("xitca-server-worker-{idx}"));

                            let task = || {
                                let on_start_fut = on_worker_start();

                                async {
                                    on_start_fut.await;

                                    let mut handles = Vec::new();
                                    let mut services = Vec::new();

                                    for (name, factory) in factories.iter() {
                                        let (h, s) = factory.call((name, &listeners)).await?;
                                        handles.extend(h);
                                        services.push(s);
                                    }

                                    worker::wait_for_stop(handles, services, shutdown_timeout, &is_graceful_shutdown)
                                        .await;

                                    Ok::<_, ()>(())
                                }
                            };

                            let handle = {
                                #[cfg(not(feature = "io-uring"))]
                                {
                                    let rt = tokio::runtime::Builder::new_current_thread()
                                        .enable_all()
                                        .max_blocking_threads(worker_max_blocking_threads)
                                        .build()?;

                                    thread.spawn_scoped(scope, move || {
                                        rt.block_on(tokio::task::LocalSet::new().run_until(task()))
                                    })?
                                }

                                #[cfg(feature = "io-uring")]
                                {
                                    thread.spawn_scoped(scope, move || {
                                        let _ = worker_max_blocking_threads;
                                        tokio_uring::start(task())
                                    })?
                                }
                            };

                            handles.push(handle);
                        }

                        Ok::<_, io::Error>(handles)
                    };

                    match spawner(s) {
                        Ok(handles) => {
                            for handle in handles {
                                let _ = handle.join();
                            }
                        }
                        Err(_) => todo!("block main thread and wait for runtime and thread builder?"),
                    }
                })
            })?;

        let (tx_cmd, rx_cmd) = tokio::sync::mpsc::unbounded_channel();

        Ok(Self {
            is_graceful_shutdown,
            tx_cmd,
            rx_cmd,
            rt: Some(rt),
            worker_join_handles: vec![worker_handles],
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
