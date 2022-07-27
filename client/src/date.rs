use std::{ops::Deref, sync::Arc, sync::RwLock, time::Duration};

use tokio::{
    task::{spawn, JoinHandle},
    time::{interval, Instant},
};
use xitca_http::date::{DateTime, DateTimeState, DATE_VALUE_LENGTH};

pub(crate) struct DateTimeService {
    state: Arc<RwLock<DateTimeState>>,
    handle: JoinHandle<()>,
}

impl Drop for DateTimeService {
    fn drop(&mut self) {
        // stop the timer update async task on drop.
        self.handle.abort();
    }
}

pub(crate) struct DateTimeHandle<'a>(&'a RwLock<DateTimeState>);

impl Deref for DateTimeHandle<'_> {
    type Target = RwLock<DateTimeState>;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl DateTimeService {
    pub(crate) fn new() -> Self {
        // shared date and timer for Date and update async task.
        let state = Arc::new(RwLock::new(DateTimeState::new()));
        let state_clone = Arc::clone(&state);
        // spawn an async task sleep for 500 milli sec and update date in a loop.
        // handle is used to stop the task on Date drop.
        let handle = spawn(async move {
            let mut interval = interval(Duration::from_millis(500));
            let state = &*state_clone;
            loop {
                let _ = interval.tick().await;
                *state.write().unwrap() = DateTimeState::new();
            }
        });

        Self { state, handle }
    }

    pub(crate) fn handle(&self) -> DateTimeHandle<'_> {
        DateTimeHandle(self.state.deref())
    }
}

impl DateTime for DateTimeHandle<'_> {
    const DATE_VALUE_LENGTH: usize = DATE_VALUE_LENGTH;

    fn with_date<F, O>(&self, f: F) -> O
    where
        F: FnOnce(&[u8]) -> O,
    {
        let state = self.read().unwrap();
        f(&state.date[..])
    }

    fn now(&self) -> Instant {
        self.read().unwrap().now
    }
}
