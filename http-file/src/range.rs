//! single byte range parsing for the `Range` request header.
//!
//! only one range is ever served so the multipart/byteranges machinery of a general
//! purpose range parser is not needed. this is also the security sensitive part of file
//! serving where an out of bound or reversed range must never reach the file system, so
//! it is kept local and directly tested.

/// outcome of parsing a `Range` header against a known file size.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Range {
    /// inclusive byte range already validated to be within the file.
    Bytes { start: u64, end: u64 },
    /// syntactically valid but not satisfiable for this file size. must produce a 416.
    Unsatisfiable,
}

/// parse the first satisfiable byte range out of a `Range` header value.
///
/// returns [None] when the header is malformed or carries a unit other than `bytes`. per
/// [RFC9110 section 14.2] such a header is ignored and the whole file is served, so the
/// caller must not turn this into an error response.
///
/// when the header lists multiple ranges the first satisfiable one is used. serving every
/// requested range would need a `multipart/byteranges` body and a server is always free to
/// answer with a single range instead.
///
/// [RFC9110 section 14.2]: https://www.rfc-editor.org/rfc/rfc9110#section-14.2
pub(super) fn parse(header: &str, size: u64) -> Option<Range> {
    // range-unit is case insensitive. bytes is the only unit that has to be supported.
    let (unit, set) = header.trim().split_at_checked("bytes=".len())?;
    if !unit.eq_ignore_ascii_case("bytes=") {
        return None;
    }

    let mut first_satisfiable = None;
    let mut seen = false;

    for spec in set.split(',') {
        let spec = spec.trim();
        // empty list elements are legal legacy syntax and are skipped.
        // see RFC9110 section 5.6.1.2.
        if spec.is_empty() {
            continue;
        }
        seen = true;
        // a single invalid element invalidates the whole header rather than the element.
        match parse_spec(spec, size)? {
            range @ Range::Bytes { .. } => {
                first_satisfiable.get_or_insert(range);
            }
            Range::Unsatisfiable => {}
        }
    }

    seen.then(|| first_satisfiable.unwrap_or(Range::Unsatisfiable))
}

fn parse_spec(spec: &str, size: u64) -> Option<Range> {
    let (first, last) = spec.split_once('-')?;
    let (first, last) = (first.trim(), last.trim());

    if first.is_empty() {
        // suffix-range. -N asks for the final N bytes of the file.
        let n = digits(last)?;
        if n == 0 || size == 0 {
            return Some(Range::Unsatisfiable);
        }
        // a suffix longer than the file is clamped to the whole file rather than rejected.
        return Some(Range::Bytes {
            start: size.saturating_sub(n),
            end: size - 1,
        });
    }

    let start = digits(first)?;

    // checked before computing end so the size - 1 below can not underflow.
    if start >= size {
        return Some(Range::Unsatisfiable);
    }

    let end = if last.is_empty() {
        size - 1
    } else {
        let end = digits(last)?;
        if end < start {
            // a reversed range is invalid syntax, not an unsatisfiable range.
            return None;
        }
        core::cmp::min(end, size - 1)
    };

    Some(Range::Bytes { start, end })
}

/// parse an unsigned integer of ASCII digits only.
///
/// [u64]'s [FromStr] would also accept a leading `+` which the grammar does not allow.
/// values that do not fit in a u64 are rejected, which surfaces as an ignored header.
///
/// [FromStr]: core::str::FromStr
fn digits(input: &str) -> Option<u64> {
    if input.is_empty() || !input.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    input.parse().ok()
}

#[cfg(test)]
mod test {
    use super::*;

    fn bytes(start: u64, end: u64) -> Option<Range> {
        Some(Range::Bytes { start, end })
    }

    #[test]
    fn int_range() {
        assert_eq!(parse("bytes=0-0", 13), bytes(0, 0));
        assert_eq!(parse("bytes=2-12", 13), bytes(2, 12));
        assert_eq!(parse("bytes=0-12", 13), bytes(0, 12));
    }

    #[test]
    fn open_ended_range() {
        assert_eq!(parse("bytes=2-", 13), bytes(2, 12));
        assert_eq!(parse("bytes=0-", 13), bytes(0, 12));
        assert_eq!(parse("bytes=12-", 13), bytes(12, 12));
    }

    #[test]
    fn suffix_range() {
        assert_eq!(parse("bytes=-5", 13), bytes(8, 12));
        assert_eq!(parse("bytes=-13", 13), bytes(0, 12));
        // a suffix larger than the file yields the whole file.
        assert_eq!(parse("bytes=-100", 13), bytes(0, 12));
        // a zero length suffix can never be satisfied.
        assert_eq!(parse("bytes=-0", 13), Some(Range::Unsatisfiable));
    }

    #[test]
    fn last_pos_is_clamped_to_file() {
        assert_eq!(parse("bytes=2-100", 13), bytes(2, 12));
        assert_eq!(parse("bytes=0-18446744073709551615", 13), bytes(0, 12));
    }

    #[test]
    fn start_past_end_of_file_is_unsatisfiable() {
        assert_eq!(parse("bytes=13-", 13), Some(Range::Unsatisfiable));
        assert_eq!(parse("bytes=13-20", 13), Some(Range::Unsatisfiable));
        assert_eq!(parse("bytes=100-200", 13), Some(Range::Unsatisfiable));
    }

    #[test]
    fn empty_file_is_never_satisfiable() {
        assert_eq!(parse("bytes=0-", 0), Some(Range::Unsatisfiable));
        assert_eq!(parse("bytes=0-0", 0), Some(Range::Unsatisfiable));
        assert_eq!(parse("bytes=-1", 0), Some(Range::Unsatisfiable));
    }

    /// an ignored header must serve the full file, so these must be [None] and never an
    /// unsatisfiable range that would turn into a 416.
    #[test]
    fn malformed_header_is_ignored() {
        for header in [
            "",
            "bytes",
            "bytes=",
            "bytes=-",
            "bytes=,",
            "bytes=abc",
            "bytes=1-2-3",
            "bytes=a-2",
            "bytes=1-b",
            // reversed range.
            "bytes=10-2",
            // leading sign is not in the grammar.
            "bytes=+1-2",
            "bytes=1-+2",
            // unsupported unit.
            "items=1-2",
            "bits=1-2",
            "1-2",
            // does not fit a u64.
            "bytes=99999999999999999999999-",
        ] {
            assert_eq!(parse(header, 13), None, "{header} should have been ignored");
        }
    }

    #[test]
    fn unit_is_case_insensitive() {
        assert_eq!(parse("BYTES=2-12", 13), bytes(2, 12));
        assert_eq!(parse("Bytes=2-12", 13), bytes(2, 12));
    }

    #[test]
    fn whitespace_is_tolerated() {
        assert_eq!(parse("  bytes=2-12  ", 13), bytes(2, 12));
        assert_eq!(parse("bytes= 2 - 12 ", 13), bytes(2, 12));
        assert_eq!(parse("bytes=0-1, 4-5", 13), bytes(0, 1));
    }

    #[test]
    fn multi_range_uses_first_satisfiable() {
        assert_eq!(parse("bytes=0-1,4-5", 13), bytes(0, 1));
        // leading unsatisfiable elements are skipped over.
        assert_eq!(parse("bytes=100-200,4-5", 13), bytes(4, 5));
        // every element unsatisfiable means a 416.
        assert_eq!(parse("bytes=100-200,300-400", 13), Some(Range::Unsatisfiable));
        // one invalid element invalidates the whole header.
        assert_eq!(parse("bytes=0-1,10-2", 13), None);
    }

    #[test]
    fn empty_list_elements_are_skipped() {
        assert_eq!(parse("bytes=,2-12", 13), bytes(2, 12));
        assert_eq!(parse("bytes=2-12,", 13), bytes(2, 12));
        assert_eq!(parse("bytes=2-12,,4-5", 13), bytes(2, 12));
    }

    /// the invariant the file system code relies on: a returned range is always inside the
    /// file and never reversed.
    #[test]
    fn returned_range_is_always_in_bounds() {
        let sizes = [0u64, 1, 2, 13, 4096];
        let specs = [
            "0-0", "0-", "-0", "-1", "-2", "-4096", "1-1", "1-0", "2-1", "0-4095", "0-99999", "4095-", "4096-", "1-2",
            "3-3",
        ];

        for size in sizes {
            for spec in specs {
                let header = format!("bytes={spec}");
                if let Some(Range::Bytes { start, end }) = parse(&header, size) {
                    assert!(start <= end, "{header} on size {size} produced a reversed range");
                    assert!(end < size, "{header} on size {size} produced an out of bound range");
                }
            }
        }
    }
}
