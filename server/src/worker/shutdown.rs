use std::{
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use tracing::info;

use super::{ServiceAny, with_worker_name_str};

pub(super) struct ShutdownHandle<'a> {
    shutdown_timeout: Duration,
    services: Vec<ServiceAny>,
    is_graceful_shutdown: &'a AtomicBool,
}

impl Drop for ShutdownHandle<'_> {
    fn drop(&mut self) {
        self.retain_active_services();

        let remaining = std::mem::take(&mut self.services)
            .into_iter()
            .fold(0, |total, service| total + Rc::strong_count(&service).saturating_sub(1));

        if remaining == 0 {
            with_worker_name_str(|name| info!("Graceful stopped {name}"));
        } else {
            with_worker_name_str(|name| info!("Force stopped {name}. {remaining} connections(estimate) left."));
        }
    }
}

impl<'a> ShutdownHandle<'a> {
    pub(super) fn new(
        shutdown_timeout: Duration,
        services: Vec<ServiceAny>,
        is_graceful_shutdown: &'a AtomicBool,
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
                self.retain_active_services();

                if self.services.is_empty() {
                    return;
                }

                let _ = interval.tick().await;
            }
        }
    }

    #[inline(never)]
    fn retain_active_services(&mut self) {
        self.services.retain(|service| Rc::strong_count(service) > 1);
    }
}
