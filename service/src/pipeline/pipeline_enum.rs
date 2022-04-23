use core::fmt::{self, Debug, Display, Formatter};

/// A pipeline type where two variants have a parent-child/first-second relationship
pub enum Pipeline<F, S> {
    First(F),
    Second(S),
}

impl<F, S> Clone for Pipeline<F, S>
where
    F: Clone,
    S: Clone,
{
    fn clone(&self) -> Self {
        match *self {
            Self::First(ref p) => Self::First(p.clone()),
            Self::Second(ref p) => Self::Second(p.clone()),
        }
    }
}

impl<F, S> Pipeline<F, S>
where
    F: From<S>,
{
    pub fn into_first(self) -> F {
        match self {
            Self::First(f) => f,
            Self::Second(s) => F::from(s),
        }
    }
}

impl<F, S> Pipeline<F, S>
where
    S: From<F>,
{
    pub fn into_second(self) -> S {
        match self {
            Self::First(f) => S::from(f),
            Self::Second(s) => s,
        }
    }
}

impl<F, S> Debug for Pipeline<F, S>
where
    F: Debug,
    S: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::First(ref p) => write!(f, "{:?}", p),
            Self::Second(ref p) => write!(f, "{:?}", p),
        }
    }
}

impl<F, S> Display for Pipeline<F, S>
where
    F: Display,
    S: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::First(ref p) => write!(f, "{}", p),
            Self::Second(ref p) => write!(f, "{}", p),
        }
    }
}
