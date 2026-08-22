//! low resolution async date time for reduced syscall for generating http date time.

use core::{
    cell::RefCell,
    fmt::{self, Write},
    ops::Deref,
    time::Duration,
};

use std::{rc::Rc, time::SystemTime};

use tokio::{
    task::JoinHandle,
    time::{Instant, interval},
};

use crate::http::header::HeaderValue;

// The length of byte representation of HttpDate
const DATE_VALUE_LENGTH: usize = 29;

const DAY: [&[u8; 3]; 7] = [b"Sun", b"Mon", b"Tue", b"Wed", b"Thu", b"Fri", b"Sat"];

const MONTH: [&[u8; 3]; 12] = [
    b"Jan", b"Feb", b"Mar", b"Apr", b"May", b"Jun", b"Jul", b"Aug", b"Sep", b"Oct", b"Nov", b"Dec",
];

/// the fixed bytes of an IMF-fixdate. `Display` overwrites only the variable fields, so
/// the separators and the trailing `GMT` never have to be written at runtime.
const TEMPLATE: [u8; DATE_VALUE_LENGTH] = *b"Xxx, 00 Xxx 0000 00:00:00 GMT";

/// write `n` as two ASCII digits at offset `OFF`. `n` is in range at every call site.
///
/// taking the whole buffer as a fixed size array turns an out of range `OFF` into a
/// compile error instead of a runtime panic.
const fn two_digits<const OFF: usize>(buf: &mut [u8; DATE_VALUE_LENGTH], n: u32) {
    const {
        assert!(
            OFF + 1 < DATE_VALUE_LENGTH,
            "two_digits would write past the date buffer"
        )
    }
    buf[OFF] = b'0' + (n / 10) as u8;
    buf[OFF + 1] = b'0' + (n % 10) as u8;
}

/// [IMF-fixdate] formatting of a [SystemTime].
///
/// a server only ever emits this one format and never parses a date, so the full date
/// handling of a dedicated crate is not needed for it.
///
/// [IMF-fixdate]: https://www.rfc-editor.org/rfc/rfc9110#section-5.6.7
struct HttpDate {
    /// seconds relative to the unix epoch. negative for dates before 1970.
    secs: i64,
}

impl From<SystemTime> for HttpDate {
    fn from(time: SystemTime) -> Self {
        let secs = match time.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(dur) => dur.as_secs() as i64,
            Err(e) => -(e.duration().as_secs() as i64),
        };
        Self { secs }
    }
}

impl fmt::Display for HttpDate {
    /// always writes exactly [DATE_VALUE_LENGTH] bytes.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // euclidean division keeps the time of day positive for pre epoch timestamps.
        let days = self.secs.div_euclid(86400);
        let rem = self.secs.rem_euclid(86400);

        // Howard Hinnant's civil_from_days. the year is shifted to start in march so the
        // leap day lands at the end of it.
        let z = days + 719468;
        let era = z.div_euclid(146097);
        let doe = z - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
        let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };

        // 1970-01-01 was a thursday, which is index 4 of DAY.
        let week_day = DAY[(days + 4).rem_euclid(7) as usize];

        let mut buf = TEMPLATE;

        buf[..3].copy_from_slice(week_day);
        two_digits::<5>(&mut buf, day);
        buf[8..11].copy_from_slice(MONTH[month as usize - 1]);

        // a year outside four digits is not representable. clamping keeps the output
        // length fixed, which callers copy into fixed size buffers.
        let year = year.clamp(0, 9999) as u32;
        two_digits::<12>(&mut buf, year / 100);
        two_digits::<14>(&mut buf, year % 100);
        two_digits::<17>(&mut buf, (rem / 3600) as u32);
        two_digits::<20>(&mut buf, (rem % 3600 / 60) as u32);
        two_digits::<23>(&mut buf, (rem % 60) as u32);

        // every byte written above is ASCII, and the whole buffer is written in one call
        // because `DateTimeState` copies it into a fixed size array.
        f.write_str(core::str::from_utf8(&buf).expect("date buffer is always ASCII"))
    }
}

/// Trait for getting current date/time.
///
/// This is usually used by a low resolution of timer to reduce frequent syscall to OS.
pub trait DateTime {
    /// The size hint of slice by Self::date method.
    const DATE_SIZE_HINT: usize = DATE_VALUE_LENGTH;

    /// closure would receive byte slice representation of [HttpDate].
    fn with_date<F, O>(&self, f: F) -> O
    where
        F: FnOnce(&[u8]) -> O;

    fn with_date_header<F, O>(&self, f: F) -> O
    where
        F: FnOnce(&HeaderValue) -> O;

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
        let state = Rc::new(RefCell::new(DateTimeState::default()));
        let state_clone = Rc::clone(&state);
        // spawn an async task sleep for 1 sec and update date in a loop.
        // handle is used to stop the task on Date drop.
        let handle = tokio::task::spawn_local(async move {
            let mut interval = interval(Duration::from_millis(500));
            loop {
                let _ = interval.tick().await;
                *state_clone.borrow_mut() = DateTimeState::default();
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

/// struct contains byte representation of [HttpDate] and [Instant].
#[derive(Clone)]
pub struct DateTimeState {
    pub date: [u8; DATE_VALUE_LENGTH],
    pub date_header: HeaderValue,
    pub now: Instant,
}

impl Default for DateTimeState {
    fn default() -> Self {
        let mut date = Self {
            date: [0; DATE_VALUE_LENGTH],
            date_header: HeaderValue::from_static(""),
            now: Instant::now(),
        };
        let _ = write!(date, "{}", HttpDate::from(SystemTime::now()));
        date.date_header = HeaderValue::from_bytes(&date.date).unwrap();
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
    // TODO: remove this allow
    #[inline]
    fn with_date<F, O>(&self, f: F) -> O
    where
        F: FnOnce(&[u8]) -> O,
    {
        let date = self.borrow();
        f(&date.date[..])
    }

    #[inline]
    fn with_date_header<F, O>(&self, f: F) -> O
    where
        F: FnOnce(&HeaderValue) -> O,
    {
        let date = self.borrow();
        f(&date.date_header)
    }

    #[inline(always)]
    fn now(&self) -> Instant {
        self.borrow().now
    }
}

/// Time handler powered by plain OS system time. useful for testing purpose.
pub struct SystemTimeDateTimeHandler;

impl DateTime for SystemTimeDateTimeHandler {
    // TODO: remove this allow
    #[allow(dead_code)]
    fn with_date<F, O>(&self, f: F) -> O
    where
        F: FnOnce(&[u8]) -> O,
    {
        let date = HttpDate::from(SystemTime::now()).to_string();
        f(date.as_bytes())
    }

    #[allow(dead_code)]
    fn with_date_header<F, O>(&self, f: F) -> O
    where
        F: FnOnce(&HeaderValue) -> O,
    {
        self.with_date(|date| {
            let val = HeaderValue::from_bytes(date).unwrap();
            f(&val)
        })
    }

    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn imf_fixdate_vectors() {
        let format = |secs: i64| HttpDate { secs }.to_string();

        // the reference timestamp from RFC9110 section 5.6.7.
        assert_eq!(format(784_111_777), "Sun, 06 Nov 1994 08:49:37 GMT");
        assert_eq!(format(0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(format(2_147_483_647), "Tue, 19 Jan 2038 03:14:07 GMT");
        assert_eq!(format(4_102_444_800), "Fri, 01 Jan 2100 00:00:00 GMT");
        // 2000 is a leap year, so 29 Feb exists.
        assert_eq!(format(951_782_400), "Tue, 29 Feb 2000 00:00:00 GMT");
        assert_eq!(format(-1), "Wed, 31 Dec 1969 23:59:59 GMT");
    }

    /// `DateTimeState::write_str` copies into a fixed size array, so a formatted date that
    /// is not exactly [DATE_VALUE_LENGTH] bytes would panic at runtime.
    #[test]
    fn formatted_length_is_always_the_date_value_length() {
        // roughly every 11 hours from 1970 to 2118, plus every second of one day.
        let steps = (0..100_000).map(|i| i * 40_000).chain(0..86_400);
        for secs in steps {
            let date = HttpDate { secs }.to_string();
            assert_eq!(date.len(), DATE_VALUE_LENGTH, "{date}");
        }
    }

    #[test]
    fn date_time_state_default_is_a_valid_header() {
        let state = DateTimeState::default();
        assert_eq!(state.date.len(), DATE_VALUE_LENGTH);
        assert_eq!(state.date_header.as_bytes(), &state.date[..]);
        assert!(state.date.ends_with(b" GMT"));
    }
}
