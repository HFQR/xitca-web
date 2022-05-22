use std::{
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use tracing::info;

use super::{worker_name, ServiceAny};

pub(super) struct ShutdownHandle {
    shutdown_timeout: Duration,
    services: Vec<ServiceAny>,
    is_graceful_shutdown: Arc<AtomicBool>,
}

impl Drop for ShutdownHandle {
    fn drop(&mut self) {
        let remaining = std::mem::take(&mut self.services)
            .into_iter()
            .fold(0, |total, service| total + Rc::strong_count(&service));

        if remaining == 0 {
            info!("Graceful stopped {}", worker_name());
        } else {
            info!(
                "Force stopped {}. {:?} connections(estimate) left.",
                worker_name(),
                remaining
            );
        }
    }
}

impl ShutdownHandle {
    pub(super) fn new(
        shutdown_timeout: Duration,
        services: Vec<ServiceAny>,
        is_graceful_shutdown: Arc<AtomicBool>,
    ) -> Self {
        Self {
            shutdown_timeout,
            services,
            is_graceful_shutdown,
        }
    }

    pub(super) async fn shutdown(mut self) {
        if self.is_graceful_shutdown.load(Ordering::SeqCst) {
            let start = Instant::now();
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            while start.elapsed() < self.shutdown_timeout {
                self.services.retain(|service| Rc::strong_count(service) > 1);

                if self.services.is_empty() {
                    return;
                }

                let _ = interval.tick().await;
            }
        }
    }
}
