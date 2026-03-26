#![allow(dead_code)]

mod data;
mod dispatcher;
mod error;
mod go_away;
mod head;
mod headers;
mod hpack;
mod ping;
mod priority;
mod reason;
mod reset;
mod settings;
mod stream_id;
mod window_update;

pub(crate) use dispatcher::Dispatcher;

const HEADER_LEN: usize = 9;

const PREFACE: &[u8; 24] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

#[cfg(feature = "io-uring")]
pub mod dispatcher_uring;

/// A helper macro that unpacks a sequence of 4 bytes found in the buffer with
/// the given identifier, starting at the given offset, into the given integer
/// type. Obviously, the integer type should be able to support at least 4
/// bytes.
///
/// # Examples
///
/// ```ignore
/// # // We ignore this doctest because the macro is not exported.
/// let buf: [u8; 4] = [0, 0, 0, 1];
/// assert_eq!(1u32, unpack_octets_4!(buf, 0, u32));
/// ```
macro_rules! unpack_octets_4 {
    // TODO: Get rid of this macro
    ($buf:expr, $offset:expr, $tip:ty) => {
        (($buf[$offset + 0] as $tip) << 24)
            | (($buf[$offset + 1] as $tip) << 16)
            | (($buf[$offset + 2] as $tip) << 8)
            | (($buf[$offset + 3] as $tip) << 0)
    };
}

use unpack_octets_4;

#[cfg(test)]
mod tests {
    #[test]
    fn test_unpack_octets_4() {
        let buf: [u8; 4] = [0, 0, 0, 1];
        assert_eq!(1u32, unpack_octets_4!(buf, 0, u32));
    }
}
