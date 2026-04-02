use super::{IoBuf, IoBufMut, Slice};

use core::{ops, ptr, slice};

/// A possibly bounded view into an owned [`IoBuf`] buffer.
///
/// Because buffers are passed by ownership to the runtime, Rust's slice API
/// (`&buf[..]`) cannot be used. Instead, `tokio-uring` provides an owned slice
/// API: [`.slice()`]. The method takes ownership of the buffer and returns a
/// [`Slice`] value that tracks the requested range.
///
/// This trait provides a generic way to use buffers and `Slice` views
/// into such buffers with `io-uring` operations.
///
/// [`.slice()`]: BoundedBuf::slice
pub trait BoundedBuf: Unpin + 'static {
    /// The type of the underlying buffer.
    type Buf: IoBuf;

    /// The type representing the range bounds of the view.
    type Bounds: ops::RangeBounds<usize>;

    /// Returns a view of the buffer with the specified range.
    ///
    /// This method is similar to Rust's slicing (`&buf[..]`), but takes
    /// ownership of the buffer. The range bounds are specified against
    /// the possibly offset beginning of the `self` view into the buffer
    /// and the end bound, if specified, must not exceed the view's total size.
    /// Note that the range may extend into the uninitialized part of the
    /// buffer, but it must start (if so bounded) in the initialized part
    /// or immediately adjacent to it.
    ///
    /// # Panics
    ///
    /// If the range is invalid with regard to the recipient's total size or
    /// the length of its initialized part, the implementation of this method
    /// should panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use tokio_uring_xitca::buf::BoundedBuf;
    ///
    /// let buf = b"hello world".to_vec();
    /// let slice = buf.slice(5..10);
    /// assert_eq!(&slice[..], b" worl");
    /// let slice = slice.slice(1..3);
    /// assert_eq!(&slice[..], b"wo");
    /// ```
    fn slice(self, range: impl ops::RangeBounds<usize>) -> Slice<Self::Buf>;

    /// Returns a `Slice` with the view's full range.
    ///
    /// This method is to be used by the `tokio-uring` runtime and it is not
    /// expected for users to call it directly.
    fn slice_full(self) -> Slice<Self::Buf>;

    /// Gets a reference to the underlying buffer.
    fn get_buf(&self) -> &Self::Buf;

    /// Returns the range bounds for this view.
    fn bounds(&self) -> Self::Bounds;

    /// Constructs a view from an underlying buffer and range bounds.
    fn from_buf_bounds(buf: Self::Buf, bounds: Self::Bounds) -> Self;

    /// Like [`IoBuf::stable_ptr`],
    /// but possibly offset to the view's starting position.
    fn stable_ptr(&self) -> *const u8;

    /// Number of initialized bytes available via this view.
    fn bytes_init(&self) -> usize;

    /// Total size of the view, including uninitialized memory, if any.
    fn bytes_total(&self) -> usize;

    /// Returns a shared reference to the initialized portion of the buffer.
    fn chunk(&self) -> &[u8] {
        // Safety: BoundedBuf implementor guarantees stable_ptr points to valid
        // memory and bytes_init bytes starting from that pointer are initialized.
        unsafe { slice::from_raw_parts(self.stable_ptr(), self.bytes_init()) }
    }
}

impl<T: IoBuf> BoundedBuf for T {
    type Buf = Self;
    type Bounds = ops::RangeFull;

    fn slice(self, range: impl ops::RangeBounds<usize>) -> Slice<Self> {
        use ops::Bound;

        let begin = match range.start_bound() {
            Bound::Included(&n) => n,
            Bound::Excluded(&n) => n.checked_add(1).expect("out of range"),
            Bound::Unbounded => 0,
        };

        assert!(begin < self.bytes_total());

        let end = match range.end_bound() {
            Bound::Included(&n) => n.checked_add(1).expect("out of range"),
            Bound::Excluded(&n) => n,
            Bound::Unbounded => self.bytes_total(),
        };

        assert!(end <= self.bytes_total());
        assert!(begin <= self.bytes_init());

        Slice::new(self, begin, end)
    }

    fn slice_full(self) -> Slice<Self> {
        let end = self.bytes_total();
        Slice::new(self, 0, end)
    }

    fn get_buf(&self) -> &Self {
        self
    }

    fn bounds(&self) -> Self::Bounds {
        ..
    }

    fn from_buf_bounds(buf: Self, _: ops::RangeFull) -> Self {
        buf
    }

    fn stable_ptr(&self) -> *const u8 {
        IoBuf::stable_ptr(self)
    }

    fn bytes_init(&self) -> usize {
        IoBuf::bytes_init(self)
    }

    fn bytes_total(&self) -> usize {
        IoBuf::bytes_total(self)
    }
}

/// A possibly bounded view into an owned [`IoBufMut`] buffer.
///
/// This trait provides a generic way to use mutable buffers and `Slice` views
/// into such buffers with `io-uring` operations.
pub trait BoundedBufMut: BoundedBuf<Buf = Self::BufMut> {
    /// The type of the underlying buffer.
    type BufMut: IoBufMut;

    /// Like [`IoBufMut::stable_mut_ptr`],
    /// but possibly offset to the view's starting position.
    fn stable_mut_ptr(&mut self) -> *mut u8;

    /// Like [`IoBufMut::set_init`],
    /// but the position is possibly offset to the view's starting position.
    ///
    /// # Safety
    ///
    /// The caller must ensure that all bytes starting at `stable_mut_ptr()` up
    /// to `pos` are initialized and owned by the buffer.
    unsafe fn set_init(&mut self, pos: usize);

    /// Copies the given byte slice into the buffer, starting at
    /// this view's offset.
    ///
    /// # Panics
    ///
    /// If the slice's length exceeds the destination's remaining capacity,
    /// this method panics.
    fn put_slice(&mut self, src: &[u8]) {
        let init = self.bytes_init();
        assert!(self.bytes_total() - init >= src.len());

        // Safety:
        // dst pointer validity is ensured by stable_mut_ptr, offset by
        // bytes_init() to write after already-initialized data;
        // the length is checked to not exceed the remaining capacity;
        // src (immutable) and dst (mutable) cannot point to overlapping memory;
        // after copying, the new initialized watermark is set accordingly.
        unsafe {
            let dst = self.stable_mut_ptr().add(init);
            ptr::copy_nonoverlapping(src.as_ptr(), dst, src.len());
            self.set_init(init + src.len());
        }
    }
}

impl<T: IoBufMut> BoundedBufMut for T {
    type BufMut = T;

    fn stable_mut_ptr(&mut self) -> *mut u8 {
        IoBufMut::stable_mut_ptr(self)
    }

    unsafe fn set_init(&mut self, pos: usize) {
        // # Safety
        //
        // implementor of T must make sure it's soundness
        unsafe { IoBufMut::set_init(self, pos) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_slice_appends() {
        let mut buf = Vec::with_capacity(64);
        buf.put_slice(b"hello");
        assert_eq!(&buf, b"hello");

        buf.put_slice(b" world");
        assert_eq!(&buf, b"hello world");
    }

    #[test]
    fn put_slice_empty() {
        let mut buf = Vec::with_capacity(16);
        buf.put_slice(b"");
        assert!(buf.is_empty());

        buf.put_slice(b"abc");
        assert_eq!(&buf, b"abc");

        buf.put_slice(b"");
        assert_eq!(&buf, b"abc");
    }

    #[test]
    fn put_slice_fills_capacity() {
        let mut buf = Vec::with_capacity(5);
        buf.put_slice(b"ab");
        buf.put_slice(b"cde");
        assert_eq!(&buf, b"abcde");
        assert_eq!(buf.len(), 5);
    }

    #[test]
    #[should_panic]
    fn put_slice_exceeds_capacity() {
        let mut buf = Vec::with_capacity(4);
        buf.put_slice(b"abcde");
    }

    #[test]
    fn chunk_returns_initialized() {
        let buf = b"hello".to_vec();
        assert_eq!(buf.chunk(), b"hello");
    }

    #[test]
    fn chunk_empty() {
        let buf = Vec::<u8>::with_capacity(16);
        assert_eq!(buf.chunk(), b"");
    }

    #[test]
    fn chunk_after_put_slice() {
        let mut buf = Vec::with_capacity(32);
        buf.put_slice(b"foo");
        buf.put_slice(b"bar");
        assert_eq!(buf.chunk(), b"foobar");
    }
}
