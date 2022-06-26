mod shutdown;

use std::{
    any::Any,
    io,
    rc::Rc,
    sync::{atomic::AtomicBool, Arc},
    thread,
    time::Duration,
};

use tokio::{task::JoinHandle, time::sleep};
use tracing::{error, info};
use xitca_io::net::{Listener, Stream};
use xitca_service::ready::ReadyService;

use self::shutdown::ShutdownHandle;

// erase Rc<S: ReadyService<_>> type and only use it for counting the reference counter of Rc.
pub(crate) type ServiceAny = Rc<dyn Any>;

pub(crate) fn start<S, Req>(listener: &Arc<Listener>, service: &S) -> JoinHandle<()>
where
    S: ReadyService<Req> + Clone + 'static,
    S::Ready: 'static,
    Req: From<Stream>,
{
    let listener = listener.clone();
    let service = service.clone();

    tokio::task::spawn_local(async move {
        loop {
            let ready = service.ready().await;

            match listener.accept().await {
                Ok(stream) => {
                    let service = service.clone();
                    tokio::task::spawn_local(async move {
                        let _ = service.call(From::from(stream)).await;
                        drop(ready);
                    });
                }
                Err(ref e) if connection_error(e) => continue,
                // TODO: This error branch is used to detect Accept thread exit.
                // Should use other notifier other than error.
                Err(ref e) if fatal_error(e) => return,
                Err(e) => {
                    error!("Error accepting connection: {e}");
                    sleep(Duration::from_secs(1)).await;
                }
            }
        }
    })
}

pub(crate) async fn wait_for_stop(
    handles: Vec<JoinHandle<()>>,
    services: Vec<ServiceAny>,
    shutdown_timeout: Duration,
    is_graceful_shutdown: &AtomicBool,
) {
    with_worker_name_str(|name| info!("Started {name}"));

    let shutdown_handle = ShutdownHandle::new(shutdown_timeout, services, is_graceful_shutdown);

    for handle in handles {
        handle
            .await
            .unwrap_or_else(|e| with_worker_name_str(|name| error!("{name} exit on error: {e}")));
    }

    shutdown_handle.shutdown().await;
}

#[cold]
#[inline(never)]
fn with_worker_name_str<F, O>(func: F) -> O
where
    F: FnOnce(&str) -> O,
{
    match thread::current().name() {
        Some(name) => func(name),
        None => func("xitca-server-worker"),
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
