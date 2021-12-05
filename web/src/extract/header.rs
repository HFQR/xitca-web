// comment out for requiring nightly unstable feature adt_const_params
// use std::{convert::Infallible, fmt, future::Future, ops::Deref};
//
// use xitca_http::util::service::FromRequest;
//
// use crate::{
//     http::header::{HeaderName, HeaderValue},
//     request::WebRequest,
// };
//
// pub struct HeaderRef<'a, const HEADER_NAME: HeaderName>(&'a HeaderValue);
//
// impl<const HEADER_NAME: HeaderName> fmt::Debug for HeaderRef<'_, HEADER_NAME> {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//         f.debug_struct("Header")
//             .field("name", &HEADER_NAME)
//             .field("value", &self.0)
//             .finish()
//     }
// }
//
// impl<const HEADER_NAME: HeaderName> Deref for HeaderRef<'_, HEADER_NAME> {
//     type Target = HeaderValue;
//
//     fn deref(&self) -> &Self::Target {
//         &self.0
//     }
// }
//
// impl<'a, 'r, 's, S, const HEADER_NAME: HeaderName> FromRequest<'a, &'r mut WebRequest<'s, S>>
//     for HeaderRef<'a, HEADER_NAME>
// {
//     type Type<'b> = HeaderRef<'b, HEADER_NAME>;
//     type Error = Infallible;
//     type Future = impl Future<Output = Result<Self, Self::Error>> + 'a;
//
//     #[inline]
//     fn from_request(req: &'a &'r mut WebRequest<'s, S>) -> Self::Future {
//         async move { Ok(HeaderRef(req.req().headers().get(HEADER_NAME).unwrap())) }
//     }
// }
