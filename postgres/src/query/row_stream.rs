use core::ops::Range;

use crate::driver::Response;

pub struct GenericRowStream<C> {
    pub(super) res: Response,
    pub(super) col: C,
    pub(super) ranges: Vec<Option<Range<usize>>>,
}
