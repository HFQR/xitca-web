#[inline]
#[cold]
fn cold() {}

#[inline]
pub(crate) fn unlikely() {
    cold();
}
