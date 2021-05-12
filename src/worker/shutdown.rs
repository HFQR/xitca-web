use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use log::info;

use super::limit::Limit;

pub(super) struct ShutdownHandle {
    shutdown_timeout: Duration,
    limit: Limit,
    is_graceful_shutdown: Arc<AtomicBool>,
    fulfilled: bool,
}

impl Drop for ShutdownHandle {
    fn drop(&mut self) {
        let id = thread::current().id();
        if self.fulfilled {
            info!("Graceful stopped worker on {:?}", id);
        } else {
            info!(
                "Force stopped worker on {:?}. {:?} connections left.",
                id,
                self.limit.get()
            );
        }
    }
}

impl ShutdownHandle {
    pub(super) fn new(
        shutdown_timeout: Duration,
        limit: Limit,
        is_graceful_shutdown: Arc<AtomicBool>,
    ) -> Self {
        info!("Started worker on {:?}", thread::current().id());
        Self {
            shutdown_timeout,
            limit,
            is_graceful_shutdown,
            fulfilled: false,
        }
    }

    pub(super) async fn shutdown(mut self) {
        if self.is_graceful_shutdown.load(Ordering::SeqCst) {
            let start = Instant::now();
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            while start.elapsed() < self.shutdown_timeout {
                if self.limit.get() == 0 {
                    return self.fulfilled = true;
                }
                let _ = interval.tick().await;
            }
        }
    }
}
