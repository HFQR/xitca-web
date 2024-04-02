use core::ops::Range;

use crate::driver::codec::Response;

pub struct GenericRowStream<C> {
    pub(crate) res: Response,
    pub(crate) col: C,
    pub(crate) ranges: Vec<Option<Range<usize>>>,
}
