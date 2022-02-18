pub struct PipelineReady<R, R1> {
    first: R,
    second: R1,
}

impl<R, R1> PipelineReady<R, R1> {
    pub(crate) fn new(first: R, second: R1) -> Self {
        Self { first, second }
    }

    #[inline]
    pub fn into_parts(self) -> (R, R1) {
        (self.first, self.second)
    }
}
