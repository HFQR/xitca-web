mod limit;
mod service;
mod shutdown;

pub(crate) use self::service::{RcWorkerService, WorkerService};

use std::{
    future::Future,
    io,
    pin::Pin,
    sync::{atomic::AtomicBool, Arc},
    task::{Context, Poll},
    thread,
    time::Duration,
};

use futures_core::ready;
use log::{error, info};
use pin_project_lite::pin_project;
use tokio::{task::JoinHandle, time::sleep};

use self::limit::Limit;
use self::shutdown::ShutdownHandle;

use crate::net::{Listener, Stream};

struct WorkerInner {
    listener: Arc<Listener>,
    service: RcWorkerService,
    limit: Limit,
}

impl WorkerInner {
    fn spawn_handling(self) -> JoinHandle<()> {
        tokio::task::spawn_local(async move {
            loop {
                let guard = self.limit.ready().await;

                match self.accept().await {
                    Ok(stream) => self.service.call((guard, stream)),
                    Err(ref e) if connection_error(e) => continue,
                    // TODO: This error branch is used to detect Accept thread exit.
                    // Should use other notifier other than error.
                    Err(ref e) if fatal_error(e) => return,
                    Err(e) => {
                        error!("Error accepting connection: {}", e);
                        sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        })
    }

    #[inline(always)]
    async fn accept(&self) -> io::Result<Stream> {
        let fut = self.listener.accept();
        let service = &self.service;

        Accept { service, fut }.await
    }
}

pin_project! {
    struct Accept<'a, Fut> {
        service: &'a RcWorkerService,
        #[pin]
        fut: Fut
    }
}

impl<Fut> Future for Accept<'_, Fut>
where
    Fut: Future<Output = io::Result<Stream>>,
{
    type Output = Fut::Output;

    #[inline(always)]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        match ready!(this.service.poll_ready(cx)) {
            Ok(_) => this.fut.poll(cx),
            Err(_) => {
                // FIXME: poll_ready error is treated as io error and delay retry accept.
                // It should restart service instead.
                Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Service::poll_ready returns error",
                )))
            }
        }
    }
}

pub(crate) async fn run(
    listeners: Vec<(String, Arc<Listener>)>,
    services: Vec<(String, RcWorkerService)>,
    connection_limit: usize,
    shutdown_timeout: Duration,
    is_graceful_shutdown: Arc<AtomicBool>,
) {
    let limit = Limit::new(connection_limit);

    let handles = listeners
        .into_iter()
        .map(|(name, listener)| {
            let service = services
                .iter()
                .find_map(|(n, service)| {
                    if n == &name {
                        Some(service.clone())
                    } else {
                        None
                    }
                })
                .unwrap();

            let listener = WorkerInner {
                listener,
                service,
                limit: limit.clone(),
            };
            listener.spawn_handling()
        })
        .collect::<Vec<_>>();

    // Drop services early as they are cloned and held by WorkerInner
    drop(services);

    info!("Started {}", worker_name());

    let shutdown_handle = ShutdownHandle::new(shutdown_timeout, limit, is_graceful_shutdown);

    for handle in handles {
        handle
            .await
            .unwrap_or_else(|e| error!("{} exit on error: {}", worker_name(), e));
    }

    shutdown_handle.shutdown().await;
}

fn worker_name() -> String {
    thread::current()
        .name()
        .map(ToString::to_string)
        .unwrap_or_else(|| String::from("actix-server-worker"))
}

/// This function defines errors that are per-connection. Which basically
/// means that if we get this error from `accept()` system call it means
/// next connection might be ready to be accepted.
///
/// All other errors will incur a timeout before next `accept()` is performed.
/// The timeout is useful to handle resource exhaustion errors like ENFILE
/// and EMFILE. Otherwise, could enter into tight loop.
fn connection_error(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::ConnectionRefused
        || e.kind() == io::ErrorKind::ConnectionAborted
        || e.kind() == io::ErrorKind::ConnectionReset
}

fn fatal_error(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::BrokenPipe || e.kind() == io::ErrorKind::Other
}
