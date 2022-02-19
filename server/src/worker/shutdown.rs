use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use tracing::info;

use super::counter::Counter;
use super::worker_name;

pub(super) struct ShutdownHandle {
    shutdown_timeout: Duration,
    counter: Counter,
    is_graceful_shutdown: Arc<AtomicBool>,
    fulfilled: bool,
}

impl Drop for ShutdownHandle {
    fn drop(&mut self) {
        if self.fulfilled {
            info!("Graceful stopped {}", worker_name());
        } else {
            info!(
                "Force stopped {}. {:?} connections left.",
                worker_name(),
                self.counter.get()
            );
        }
    }
}

impl ShutdownHandle {
    pub(super) fn new(shutdown_timeout: Duration, counter: Counter, is_graceful_shutdown: Arc<AtomicBool>) -> Self {
        Self {
            shutdown_timeout,
            counter,
            is_graceful_shutdown,
            fulfilled: false,
        }
    }

    pub(super) async fn shutdown(mut self) {
        if self.is_graceful_shutdown.load(Ordering::SeqCst) {
            let start = Instant::now();
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            while start.elapsed() < self.shutdown_timeout {
                if self.counter.get() == 0 {
                    return self.fulfilled = true;
                }
                let _ = interval.tick().await;
            }
        }
    }
}
