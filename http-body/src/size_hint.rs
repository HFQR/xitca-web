#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SizeHint {
    None,
    Exact(u64),
    #[default]
    Unknown,
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

    pub(crate) fn stream_size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Exact(len) => {
                let len = usize::try_from(*len).unwrap();
                (len, Some(len))
            }
            Self::Unknown => (0, None),
            Self::None => Self::NO_BODY_HINT,
        }
    }
}
