mod limit;
mod service;

pub(crate) use self::service::{RcWorkerService, WorkerService};

use std::{
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use log::error;
use tokio::{task::JoinHandle, time::sleep};

use self::limit::Limit;

use crate::net::Listener;

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

                match self.listener.accept().await {
                    Ok(stream) => {
                        self.service_ready().await;
                        let _ = self.service.call((guard, stream));
                    }
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

    fn service_ready(&self) -> ServiceReady<'_> {
        ServiceReady(&self.service)
    }
}

struct ServiceReady<'a>(&'a RcWorkerService);

impl Future for ServiceReady<'_> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // FIXME: poll_ready error is treated as pending.
        match self.0.poll_ready(cx) {
            Poll::Ready(Ok(_)) => Poll::Ready(()),
            _ => Poll::Pending,
        }
    }
}

pub(crate) async fn run(
    listeners: Vec<(String, Arc<Listener>)>,
    services: Vec<(String, RcWorkerService)>,
    connection_limit: usize,
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

    for handle in handles {
        handle
            .await
            .unwrap_or_else(|e| error!("Error running Worker: {}", e));
    }
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
