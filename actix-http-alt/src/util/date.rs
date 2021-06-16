use std::{
    cell::RefCell,
    fmt::{self, Write},
    rc::Rc,
    time::{Duration, SystemTime},
};

use httpdate::HttpDate;
use tokio::{
    task::JoinHandle,
    time::{interval, Instant},
};

pub(crate) const DATE_VALUE_LENGTH: usize = 29;

pub(crate) type Date = RefCell<DateTimeInner>;
pub(crate) type SharedDate = Rc<Date>;

#[derive(Copy, Clone)]
pub(crate) struct DateTimeInner {
    date: [u8; DATE_VALUE_LENGTH],
    now: Instant,
}

impl DateTimeInner {
    fn new() -> Self {
        let mut date = Self {
            date: [0; DATE_VALUE_LENGTH],
            now: Instant::now(),
        };
        let _ = write!(&mut date, "{}", HttpDate::from(SystemTime::now()));
        date
    }

    // TODO: remove this allow
    #[allow(dead_code)]
    #[inline(always)]
    pub(crate) fn date(&self) -> &[u8] {
        &self.date[..]
    }

    #[inline(always)]
    pub(crate) fn now(&self) -> Instant {
        self.now
    }
}

impl fmt::Write for DateTimeInner {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.date[..].copy_from_slice(s.as_bytes());
        Ok(())
    }
}

/// Struct with Date update periodically at 500 milli seconds interval.
pub(crate) struct DateTimeTask {
    current: Rc<RefCell<DateTimeInner>>,
    handle: JoinHandle<()>,
}

impl Drop for DateTimeTask {
    fn drop(&mut self) {
        // stop the timer update async task on drop.
        self.handle.abort();
    }
}

impl DateTimeTask {
    pub(crate) fn new() -> Self {
        // shared date and timer for Date and update async task.
        let current = Rc::new(RefCell::new(DateTimeInner::new()));
        let current_clone = Rc::clone(&current);
        // spawn an async task sleep for 1 sec and update date in a loop.
        // handle is used to stop the task on Date drop.
        let handle = tokio::task::spawn_local(async move {
            let mut interval = interval(Duration::from_millis(500));

            loop {
                let _ = interval.tick().await;
                *(*current_clone).borrow_mut() = DateTimeInner::new();
            }
        });

        Self { current, handle }
    }

    #[inline(always)]
    pub(crate) fn get(&self) -> &Date {
        &*self.current
    }

    #[inline(always)]
    pub(crate) fn get_shared(&self) -> &SharedDate {
        &self.current
    }
}
