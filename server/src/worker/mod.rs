mod limit;
mod shutdown;

pub(crate) use self::limit::Limit;

use std::{
    io,
    sync::{atomic::AtomicBool, Arc},
    thread,
    time::Duration,
};

use tokio::{task::JoinHandle, time::sleep};
use tracing::{error, info};
use xitca_io::net::{Listener, Stream};
use xitca_service::ready::ReadyService;

use self::shutdown::ShutdownHandle;

pub(crate) fn start<S, Req>(listener: &Arc<Listener>, service: &S, limit: &Limit) -> JoinHandle<()>
where
    S: ReadyService<Req> + Clone + 'static,
    S::Ready: 'static,
    Req: From<Stream>,
{
    let listener = listener.clone();
    let limit = limit.clone();
    let service = service.clone();

    tokio::task::spawn_local(async move {
        loop {
            let guard = limit.ready().await;

            // TODO: What if service return Error when ready.
            let ready = service.ready().await.ok();

            match listener.accept().await {
                Ok(stream) => {
                    let service = service.clone();
                    tokio::task::spawn_local(async move {
                        let _ = service.call(From::from(stream)).await;
                        drop(guard);
                        drop(ready);
                    });
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

pub(crate) async fn wait_for_stop(
    handles: Vec<JoinHandle<()>>,
    shutdown_timeout: Duration,
    limit: Limit,
    is_graceful_shutdown: Arc<AtomicBool>,
) {
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
        .unwrap_or_else(|| String::from("xitca-server-worker"))
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
