/// A hasher that do hashing by not doing it.
/// This hasher does not contain unsafe Rust code but it's unsafe to use for general purpose
/// and panic at runtime for key types that can't be used.
use core::hash::{BuildHasherDefault, Hasher};

/// A simple hasher that do hashing by not doing it.
#[derive(Debug, Default, Copy, Clone)]
pub struct NoHasher(u64);

/// HashBuilder for [NoHasher]
pub type NoHashBuilder = BuildHasherDefault<NoHasher>;

impl Hasher for NoHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, _: &[u8]) {
        unreachable!("NoHasher only work for Key type that can be cast as u64")
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.0 = i as u64
    }

    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.0 = i as u64
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.0 = i as u64
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.0 = i as u64
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.0 = i as u64
    }

    #[inline]
    fn write_i8(&mut self, i: i8) {
        self.0 = i as u64
    }

    #[inline]
    fn write_i16(&mut self, i: i16) {
        self.0 = i as u64
    }

    #[inline]
    fn write_i32(&mut self, i: i32) {
        self.0 = i as u64
    }

    #[inline]
    fn write_i64(&mut self, i: i64) {
        self.0 = i as u64
    }

    #[inline]
    fn write_isize(&mut self, i: isize) {
        self.0 = i as u64
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use core::hash::BuildHasher;

    #[test]
    fn test() {
        let mut hasher = NoHashBuilder::default().build_hasher();

        hasher.write_i8(7);
        assert_eq!(hasher.finish(), 7u64);

        hasher.write_u8(251);
        assert_eq!(hasher.finish(), 251u64);

        hasher.write_i16(996);
        assert_eq!(hasher.finish(), 996u64);

        hasher.write_u16(996);
        assert_eq!(hasher.finish(), 996u64);

        hasher.write_i32(996);
        assert_eq!(hasher.finish(), 996u64);

        hasher.write_u32(996);
        assert_eq!(hasher.finish(), 996u64);

        hasher.write_i64(996);
        assert_eq!(hasher.finish(), 996u64);

        hasher.write_u64(996);
        assert_eq!(hasher.finish(), 996u64);

        hasher.write_isize(996);
        assert_eq!(hasher.finish(), 996u64);

        hasher.write_usize(996);
        assert_eq!(hasher.finish(), 996u64);
    }

    #[test]
    #[should_panic]
    fn not_support() {
        let mut hasher = NoHashBuilder::default().build_hasher();
        hasher.write_i128(996);
    }
}
