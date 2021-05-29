use std::{
    cell::Cell,
    fmt::{self, Write},
    rc::Rc,
    time::Duration,
};

use httpdate::HttpDate;
use std::time::SystemTime;
use tokio::{task::JoinHandle, time::interval};

pub(crate) const DATE_VALUE_LENGTH: usize = 29;

pub(crate) type Date = Cell<DateInner>;

#[derive(Copy, Clone)]
pub(crate) struct DateInner {
    bytes: [u8; DATE_VALUE_LENGTH],
}

impl DateInner {
    fn new() -> Self {
        let mut date = Self {
            bytes: [0; DATE_VALUE_LENGTH],
        };
        let now = SystemTime::now();
        let _ = write!(&mut date, "{}", HttpDate::from(now));
        date
    }

    #[inline(always)]
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes[..]
    }
}

impl fmt::Write for DateInner {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.bytes[..].copy_from_slice(s.as_bytes());
        Ok(())
    }
}

/// Struct with Date update periodically at 1 second interval.
pub(crate) struct DateTask {
    current: Rc<Cell<DateInner>>,
    handle: JoinHandle<()>,
}

impl Drop for DateTask {
    fn drop(&mut self) {
        // stop the timer update async task on drop.
        self.handle.abort();
    }
}

impl DateTask {
    pub(crate) fn new() -> Self {
        // shared date and timer for Date and update async task.
        let current = Rc::new(Cell::new(DateInner::new()));
        let current_clone = Rc::clone(&current);
        // spawn an async task sleep for 1 sec and update date in a loop.
        // handle is used to stop the task on Date drop.
        let handle = tokio::task::spawn_local(async move {
            let mut interval = interval(Duration::from_secs(1));

            loop {
                let _ = interval.tick().await;
                let date = DateInner::new();
                current_clone.set(date);
            }
        });

        Self { current, handle }
    }

    pub(crate) fn get(&self) -> &Date {
        &*self.current
    }
}
