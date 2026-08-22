//! http date parsing and formatting.
//!
//! serving a file needs to emit a `last-modified` header and compare `if-modified-since`
//! and `if-unmodified-since` against it. that is a small enough slice of date handling to
//! keep local rather than depend on a date crate for.

use core::{fmt, time::Duration};

use std::time::SystemTime;

/// length of an [IMF-fixdate] which is the only format this type emits.
///
/// [IMF-fixdate]: https://www.rfc-editor.org/rfc/rfc9110#section-5.6.7
pub(super) const IMF_FIXDATE_LENGTH: usize = 29;

/// a timestamp with the one second resolution http date formats carry.
///
/// ordering is chronological. two values that format to the same header are equal, which
/// is what the `if-modified-since` comparison needs: the sub second part of a file
/// modification time must not make it look newer than the date a client echoes back.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct HttpDate {
    /// seconds relative to the unix epoch. negative for dates before 1970.
    secs: i64,
}

impl From<SystemTime> for HttpDate {
    fn from(time: SystemTime) -> Self {
        let secs = match time.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(dur) => dur.as_secs() as i64,
            // a pre epoch timestamp truncates towards negative infinity so that it keeps
            // comparing as older than the second it falls in.
            Err(e) => {
                let dur = e.duration();
                let secs = dur.as_secs() as i64;
                if dur.subsec_nanos() > 0 { -secs - 1 } else { -secs }
            }
        };
        Self { secs }
    }
}

impl From<HttpDate> for SystemTime {
    fn from(date: HttpDate) -> Self {
        if date.secs >= 0 {
            SystemTime::UNIX_EPOCH + Duration::from_secs(date.secs as u64)
        } else {
            SystemTime::UNIX_EPOCH - Duration::from_secs(date.secs.unsigned_abs())
        }
    }
}

const DAY: [&[u8; 3]; 7] = [b"Sun", b"Mon", b"Tue", b"Wed", b"Thu", b"Fri", b"Sat"];
const MONTH: [&[u8; 3]; 12] = [
    b"Jan", b"Feb", b"Mar", b"Apr", b"May", b"Jun", b"Jul", b"Aug", b"Sep", b"Oct", b"Nov", b"Dec",
];

/// the fixed bytes of an IMF-fixdate. [Display] overwrites only the variable fields, so
/// the separators and the trailing `GMT` never have to be written at runtime.
///
/// [Display]: fmt::Display
const TEMPLATE: [u8; IMF_FIXDATE_LENGTH] = *b"Xxx, 00 Xxx 0000 00:00:00 GMT";

/// write `n` as two ASCII digits at offset `OFF`. `n` is in range at every call site.
///
/// taking the whole buffer as a fixed size array turns an out of range `OFF` into a
/// compile error instead of a runtime panic.
const fn two_digits<const OFF: usize>(buf: &mut [u8; IMF_FIXDATE_LENGTH], n: u32) {
    const {
        assert!(
            OFF + 1 < IMF_FIXDATE_LENGTH,
            "two_digits would write past the date buffer"
        )
    }
    buf[OFF] = b'0' + (n / 10) as u8;
    buf[OFF + 1] = b'0' + (n % 10) as u8;
}

impl fmt::Display for HttpDate {
    /// always emits [IMF_FIXDATE_LENGTH] bytes of IMF-fixdate, the only format RFC9110
    /// allows a sender to produce.
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

impl HttpDate {
    /// accepts all three formats RFC9110 section 5.6.7 requires a recipient to support.
    ///
    /// a date that can not be understood is [None]. RFC9110 section 13.1 has the caller
    /// ignore such a precondition header rather than reject the request, so there is
    /// nothing to report about why parsing failed.
    pub(super) fn parse(s: &str) -> Option<Self> {
        let s = s.as_bytes();
        // IMF-fixdate:  Sun, 06 Nov 1994 08:49:37 GMT
        // RFC 850:      Sunday, 06-Nov-94 08:49:37 GMT
        // asctime:      Sun Nov  6 08:49:37 1994
        parse_imf_fixdate(s)
            .or_else(|| parse_rfc850(s))
            .or_else(|| parse_asctime(s))
    }
}

fn parse_imf_fixdate(s: &[u8]) -> Option<HttpDate> {
    // Sun, 06 Nov 1994 08:49:37 GMT
    let &[
        _,
        _,
        _,
        b',',
        b' ',
        d0,
        d1,
        b' ',
        m0,
        m1,
        m2,
        b' ',
        y0,
        y1,
        y2,
        y3,
        b' ',
        ref time @ ..,
        b' ',
        b'G',
        b'M',
        b'T',
    ] = s
    else {
        return None;
    };
    compose(
        digits4(y0, y1, y2, y3)? as i64,
        month([m0, m1, m2])?,
        digits2(d0, d1)?,
        time_of_day(time)?,
    )
}

fn parse_rfc850(s: &[u8]) -> Option<HttpDate> {
    // Sunday, 06-Nov-94 08:49:37 GMT
    //
    // the weekday name is spelled out in full and has no fixed width, so the rest of the
    // date is located relative to the comma instead.
    let comma = s.iter().position(|b| *b == b',')?;
    let &[
        b' ',
        d0,
        d1,
        b'-',
        m0,
        m1,
        m2,
        b'-',
        y0,
        y1,
        b' ',
        ref time @ ..,
        b' ',
        b'G',
        b'M',
        b'T',
    ] = s.get(comma + 1..)?
    else {
        return None;
    };
    compose(
        // a two digit year is read as within 50 years of now, per RFC9110 section 5.6.7.
        two_digit_year(digits2(y0, y1)? as i64),
        month([m0, m1, m2])?,
        digits2(d0, d1)?,
        time_of_day(time)?,
    )
}

fn parse_asctime(s: &[u8]) -> Option<HttpDate> {
    // Sun Nov  6 08:49:37 1994
    let &[
        _,
        _,
        _,
        b' ',
        m0,
        m1,
        m2,
        b' ',
        d0,
        d1,
        b' ',
        ref time @ ..,
        b' ',
        y0,
        y1,
        y2,
        y3,
    ] = s
    else {
        return None;
    };
    compose(
        digits4(y0, y1, y2, y3)? as i64,
        month([m0, m1, m2])?,
        // day is space padded rather than zero padded in this format.
        match d0 {
            b' ' => digit(d1)?,
            _ => digits2(d0, d1)?,
        },
        time_of_day(time)?,
    )
}

/// combine a validated calendar date and time of day into seconds since the epoch.
fn compose(year: i64, month: u32, day: u32, (hour, min, sec): (u32, u32, u32)) -> Option<HttpDate> {
    if day == 0 || day > days_in_month(year, month) {
        return None;
    }
    // a leap second is folded into the last second of the minute rather than rejected.
    let sec = core::cmp::min(sec, 59);
    let secs = days_from_civil(year, month, day) * 86400 + (hour * 3600 + min * 60 + sec) as i64;
    Some(HttpDate { secs })
}

/// all three formats spell the time of day as the same eight bytes.
fn time_of_day(s: &[u8]) -> Option<(u32, u32, u32)> {
    let &[h0, h1, b':', m0, m1, b':', s0, s1] = s else {
        return None;
    };
    let hour = digits2(h0, h1)?;
    let min = digits2(m0, m1)?;
    // 60 is accepted here for a leap second and folded back by `compose`.
    let sec = digits2(s0, s1)?;
    (hour < 24 && min < 60 && sec < 61).then_some((hour, min, sec))
}

fn month(name: [u8; 3]) -> Option<u32> {
    MONTH.iter().position(|month| **month == name).map(|idx| idx as u32 + 1)
}

fn digit(b: u8) -> Option<u32> {
    b.is_ascii_digit().then(|| (b - b'0') as u32)
}

fn digits2(a: u8, b: u8) -> Option<u32> {
    Some(digit(a)? * 10 + digit(b)?)
}

fn digits4(a: u8, b: u8, c: u8, d: u8) -> Option<u32> {
    Some(digits2(a, b)? * 100 + digits2(c, d)?)
}

/// expand a two digit year to the most recent year with those digits that is not more
/// than 50 years in the future, as required for the obsolete RFC 850 format.
fn two_digit_year(year: i64) -> i64 {
    let now = HttpDate::from(SystemTime::now());
    let (current, ..) = civil_from_days(now.secs.div_euclid(86400));
    let candidate = current - current.rem_euclid(100) + year;
    if candidate > current + 50 {
        candidate - 100
    } else {
        candidate
    }
}

fn is_leap_year(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// days since the unix epoch for a proleptic gregorian date.
///
/// this and [civil_from_days] are Howard Hinnant's `days_from_civil`/`civil_from_days`,
/// which shift the year to start in march so the leap day lands at the end of it.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let month = month as i64;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// inverse of [days_from_civil]. returns `(year, month, day)`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod test {
    use super::*;

    fn date(s: &str) -> HttpDate {
        HttpDate::parse(s).unwrap()
    }

    /// the reference timestamp used throughout RFC9110 section 5.6.7.
    const REF: i64 = 784_111_777;

    #[test]
    fn parse_all_three_formats() {
        // all three spell the same instant.
        assert_eq!(date("Sun, 06 Nov 1994 08:49:37 GMT").secs, REF);
        assert_eq!(date("Sunday, 06-Nov-94 08:49:37 GMT").secs, REF);
        assert_eq!(date("Sun Nov  6 08:49:37 1994").secs, REF);
    }

    #[test]
    fn format_is_imf_fixdate() {
        let formatted = HttpDate { secs: REF }.to_string();
        assert_eq!(formatted, "Sun, 06 Nov 1994 08:49:37 GMT");
        assert_eq!(formatted.len(), IMF_FIXDATE_LENGTH);
    }

    #[test]
    fn epoch_and_boundaries() {
        assert_eq!(HttpDate { secs: 0 }.to_string(), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(date("Thu, 01 Jan 1970 00:00:00 GMT").secs, 0);
        // last second before the 2038 signed 32 bit overflow and well past it.
        assert_eq!(
            HttpDate { secs: 2_147_483_647 }.to_string(),
            "Tue, 19 Jan 2038 03:14:07 GMT"
        );
        assert_eq!(
            HttpDate { secs: 4_102_444_800 }.to_string(),
            "Fri, 01 Jan 2100 00:00:00 GMT"
        );
    }

    #[test]
    fn leap_days() {
        // 2000 is a leap year, 1900 and 2100 are not.
        assert_eq!(
            HttpDate { secs: 951_782_400 }.to_string(),
            "Tue, 29 Feb 2000 00:00:00 GMT"
        );
        assert!(HttpDate::parse("Wed, 29 Feb 1900 00:00:00 GMT").is_none());
        assert!(HttpDate::parse("Mon, 29 Feb 2100 00:00:00 GMT").is_none());
        assert!(HttpDate::parse("Sat, 29 Feb 2020 00:00:00 GMT").is_some());
    }

    /// formatting then parsing must be the identity across a wide span of dates.
    #[test]
    fn round_trip() {
        // roughly every 11 hours from 1970 to 2118, plus every second of one day.
        let steps = (0..100_000).map(|i| i * 40_000).chain(0..86_400);
        for secs in steps {
            let date = HttpDate { secs };
            let formatted = date.to_string();
            assert_eq!(formatted.len(), IMF_FIXDATE_LENGTH, "{formatted}");
            assert_eq!(HttpDate::parse(&formatted).unwrap(), date, "{formatted}");
        }
    }

    #[test]
    fn civil_conversions_are_inverse() {
        for days in (-800_000..800_000).step_by(7) {
            let (y, m, d) = civil_from_days(days);
            assert!((1..=12).contains(&m) && (1..=31).contains(&d), "{days} -> {y}-{m}-{d}");
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }

    #[test]
    fn ordering_is_chronological() {
        assert!(date("Sun, 06 Nov 1994 08:49:37 GMT") < date("Sun, 06 Nov 1994 08:49:38 GMT"));
        assert!(date("Sun, 06 Nov 1994 08:49:37 GMT") < date("Mon, 07 Nov 1994 00:00:00 GMT"));
        assert!(date("Thu, 01 Jan 1970 00:00:00 GMT") < date("Sun, 06 Nov 1994 08:49:37 GMT"));
        assert_eq!(date("Sun, 06 Nov 1994 08:49:37 GMT"), date("Sun Nov  6 08:49:37 1994"));
    }

    /// a file modification time carries sub second precision that the header format drops.
    /// truncating keeps a file from looking newer than a date derived from itself.
    #[test]
    fn system_time_truncates_towards_the_past() {
        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(REF as u64);
        assert_eq!(HttpDate::from(base).secs, REF);
        assert_eq!(HttpDate::from(base + Duration::from_nanos(999_999_999)).secs, REF);

        let pre_epoch = SystemTime::UNIX_EPOCH - Duration::from_millis(1500);
        assert_eq!(HttpDate::from(pre_epoch).secs, -2);
        assert_eq!(HttpDate::from(SystemTime::UNIX_EPOCH - Duration::from_secs(1)).secs, -1);
    }

    #[test]
    fn system_time_round_trip() {
        for secs in [0i64, 1, REF, 2_147_483_647, -1, -86_400] {
            let date = HttpDate { secs };
            assert_eq!(HttpDate::from(SystemTime::from(date)), date);
        }
    }

    #[test]
    fn rejects_malformed() {
        for s in [
            "",
            "Sun, 06 Nov 1994 08:49:37",
            "Sun, 06 Nov 1994 08:49:37 UTC",
            "Sun, 06 Nov 1994 08:49:37 GMT ",
            "Sun; 06 Nov 1994 08:49:37 GMT",
            "Sun, 06 Nex 1994 08:49:37 GMT",
            "Sun, 32 Nov 1994 08:49:37 GMT",
            "Sun, 00 Nov 1994 08:49:37 GMT",
            "Sun, 06 Nov 1994 24:49:37 GMT",
            "Sun, 06 Nov 1994 08:60:37 GMT",
            "Sun, 06 Nov 199a 08:49:37 GMT",
            "Sun, 31 Apr 1994 08:49:37 GMT",
            "Sun Nov 6 08:49:37 1994",
            "Sunday, 06-Nov-94 08:49:37 UTC",
        ] {
            assert!(HttpDate::parse(s).is_none(), "{s} should not have parsed");
        }
    }

    /// a leap second is not representable so it folds into the preceding second rather
    /// than failing the whole precondition check.
    #[test]
    fn leap_second_folds() {
        assert_eq!(
            date("Sun, 31 Dec 1995 23:59:60 GMT"),
            date("Sun, 31 Dec 1995 23:59:59 GMT")
        );
    }

    #[test]
    fn two_digit_year_is_within_fifty_years() {
        let now = HttpDate::from(SystemTime::now());
        for year in 0..100 {
            let expanded = two_digit_year(year);
            assert_eq!(expanded.rem_euclid(100), year);
            let (current, ..) = civil_from_days(now.secs.div_euclid(86400));
            assert!(
                expanded <= current + 50 && expanded > current - 50,
                "{year} expanded to {expanded}"
            );
        }
    }
}
