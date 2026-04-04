use http::header::HeaderMap;

pub enum Frame<D> {
    Data(D),
    Trailers(HeaderMap),
}

impl<D> Frame<D> {
    #[inline]
    pub fn into_data(self) -> Result<D, Self> {
        match self {
            Self::Data(data) => Ok(data),
            this => Err(this),
        }
    }

    #[inline]
    pub fn into_trailers(self) -> Result<HeaderMap, Self> {
        match self {
            Self::Trailers(trailers) => Ok(trailers),
            this => Err(this),
        }
    }

    #[inline]
    pub fn data_ref(&self) -> Option<&D> {
        match self {
            Self::Data(data) => Some(data),
            _ => None,
        }
    }

    #[inline]
    pub fn data_ref_mut(&mut self) -> Option<&mut D> {
        match self {
            Self::Data(data) => Some(data),
            _ => None,
        }
    }
}
