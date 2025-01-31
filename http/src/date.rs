//! low resolution async date time for reduced syscall for generating http date time.

use std::{
    cell::RefCell,
    fmt::{self, Write},
    ops::Deref,
    rc::Rc,
    time::{Duration, SystemTime},
};

use httpdate::HttpDate;
use tokio::{
    task::JoinHandle,
    time::{Instant, interval},
};

/// Trait for getting current date/time.
///
/// This is usually used by a low resolution of timer to reduce frequent syscall to OS.
pub trait DateTime {
    /// The size hint of slice by Self::date method.
    const DATE_VALUE_LENGTH: usize;

    /// closure would receive byte slice representation of [HttpDate].
    fn with_date<F, O>(&self, f: F) -> O
    where
        F: FnOnce(&[u8]) -> O;

    fn now(&self) -> Instant;
}

/// Struct with Date update periodically at 500 milliseconds interval.
pub struct DateTimeService {
    state: Rc<RefCell<DateTimeState>>,
    handle: JoinHandle<()>,
}

impl Drop for DateTimeService {
    fn drop(&mut self) {
        // stop the timer update async task on drop.
        self.handle.abort();
    }
}

impl Default for DateTimeService {
    fn default() -> Self {
        Self::new()
    }
}

impl DateTimeService {
    pub fn new() -> Self {
        // shared date and timer for Date and update async task.
        let state = Rc::new(RefCell::new(DateTimeState::new()));
        let state_clone = Rc::clone(&state);
        // spawn an async task sleep for 1 sec and update date in a loop.
        // handle is used to stop the task on Date drop.
        let handle = tokio::task::spawn_local(async move {
            let mut interval = interval(Duration::from_millis(500));
            let state = &*state_clone;
            loop {
                let _ = interval.tick().await;
                *state.borrow_mut() = DateTimeState::new();
            }
        });

        Self { state, handle }
    }

    #[inline]
    pub fn get(&self) -> &DateTimeHandle {
        self.state.deref()
    }
}

pub(crate) type DateTimeHandle = RefCell<DateTimeState>;

/// The length of byte representation of [HttpDate].
pub const DATE_VALUE_LENGTH: usize = 29;

/// struct contains byte representation of [HttpDate] and [Instant].
#[derive(Copy, Clone)]
pub struct DateTimeState {
    pub date: [u8; DATE_VALUE_LENGTH],
    pub now: Instant,
}

impl Default for DateTimeState {
    fn default() -> Self {
        Self::new()
    }
}

impl DateTimeState {
    pub fn new() -> Self {
        let mut date = Self {
            date: [0; DATE_VALUE_LENGTH],
            now: Instant::now(),
        };
        let _ = write!(date, "{}", HttpDate::from(SystemTime::now()));
        date
    }
}

impl Write for DateTimeState {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.date[..].copy_from_slice(s.as_bytes());
        Ok(())
    }
}

impl DateTime for DateTimeHandle {
    const DATE_VALUE_LENGTH: usize = DATE_VALUE_LENGTH;

    // TODO: remove this allow
    #[allow(dead_code)]
    #[inline]
    fn with_date<F, O>(&self, f: F) -> O
    where
        F: FnOnce(&[u8]) -> O,
    {
        let date = self.borrow();
        f(&date.date[..])
    }

    #[inline(always)]
    fn now(&self) -> Instant {
        self.borrow().now
    }
}

/// Time handler powered by plain OS system time. useful for testing purpose.
pub struct SystemTimeDateTimeHandler;

impl DateTime for SystemTimeDateTimeHandler {
    const DATE_VALUE_LENGTH: usize = DATE_VALUE_LENGTH;

    // TODO: remove this allow
    #[allow(dead_code)]
    fn with_date<F, O>(&self, f: F) -> O
    where
        F: FnOnce(&[u8]) -> O,
    {
        let date = HttpDate::from(SystemTime::now()).to_string();
        f(date.as_bytes())
    }

    fn now(&self) -> Instant {
        Instant::now()
    }
}
