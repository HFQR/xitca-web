use core::{
    fmt::{self, Debug, Display, Formatter},
    marker::PhantomData,
};

/// A pipeline type where two fields have a parent-child/first-second relationship
pub struct Pipeline<F, S, M = ()> {
    pub first: F,
    pub second: S,
    _marker: PhantomData<M>,
}

impl<F, S, M> Pipeline<F, S, M> {
    pub const fn new(first: F, second: S) -> Self {
        Self {
            first,
            second,
            _marker: PhantomData,
        }
    }
}

impl<F, S, M> Clone for Pipeline<F, S, M>
where
    F: Clone,
    S: Clone,
{
    fn clone(&self) -> Self {
        Self {
            first: self.first.clone(),
            second: self.second.clone(),
            _marker: PhantomData,
        }
    }
}

impl<F, S, M> Debug for Pipeline<F, S, M>
where
    F: Debug,
    S: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pipeline")
            .field("first", &self.first)
            .field("second", &self.second)
            .finish()
    }
}

impl<F, S, M> Display for Pipeline<F, S, M>
where
    F: Display,
    S: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}, {}", self.first, self.second)
    }
}
