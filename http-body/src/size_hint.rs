use std::ops::Add;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SizeHint {
    None,
    Exact(u64),
    #[default]
    Unknown,
}

impl Add for SizeHint {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::None, Self::None) => Self::None,
            (Self::None, Self::Exact(size)) => Self::Exact(size),
            (Self::Exact(size), Self::None) => Self::Exact(size),
            (Self::Exact(size1), Self::Exact(size2)) => size1.checked_add(size2).map_or(Self::Unknown, Self::Exact),
            _ => Self::Unknown,
        }
    }
}

impl SizeHint {
    /// a crate hack for expressing [`SizeHint::None`] through [`Stream::size_hint`]
    ///
    /// [`Stream::size_hint`]: futures_core::stream::Stream::size_hint
    pub const NO_BODY_HINT: (usize, Option<usize>) = (usize::MAX, Some(0));

    pub(crate) fn exact<U>(size: U) -> Self
    where
        u64: TryFrom<U>,
        <u64 as TryFrom<U>>::Error: core::fmt::Debug,
    {
        SizeHint::Exact(u64::try_from(size).unwrap())
    }

    pub(crate) fn as_stream_size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Exact(len) => {
                let len = usize::try_from(*len).unwrap();
                (len, Some(len))
            }
            Self::Unknown => (0, None),
            Self::None => Self::NO_BODY_HINT,
        }
    }

    pub(crate) fn from_stream_size_hint(hint: (usize, Option<usize>)) -> Self {
        match hint {
            Self::NO_BODY_HINT => Self::None,
            (low, Some(up)) if low == up => Self::exact(low),
            _ => Self::Unknown,
        }
    }
}
