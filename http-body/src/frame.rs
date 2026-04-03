use http::header::HeaderMap;

pub enum Frame<D> {
    Data(D),
    Trailers(HeaderMap),
}
