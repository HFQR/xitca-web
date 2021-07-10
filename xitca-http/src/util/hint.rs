#[inline]
#[cold]
fn cold() {}

// #[inline]
// pub(crate) fn likely(b: bool) -> bool {
//     if !b {
//         cold()
//     }
//     b
// }

#[inline]
pub(crate) fn unlikely(b: bool) -> bool {
    if b {
        cold()
    }
    b
}
