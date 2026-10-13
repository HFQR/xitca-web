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
