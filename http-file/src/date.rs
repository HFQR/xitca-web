use core::{
    fmt::{self, Write},
    str::FromStr,
};

use std::fs::Metadata;

use bytes::{Bytes, BytesMut};
use http::{
    header::{HeaderValue, IF_MODIFIED_SINCE, IF_UNMODIFIED_SINCE},
    Request,
};
use httpdate::HttpDate;

use super::error::ServeError;

pub(super) fn modified_check<Ext>(req: &Request<Ext>, md: &Metadata) -> Result<Option<HttpDate>, ServeError> {
    let modified_time = match md.modified() {
        Ok(modified) => HttpDate::from(modified),
        Err(_) => {
            #[cold]
            #[inline(never)]
            fn precondition_check<Ext>(req: &Request<Ext>) -> Result<Option<HttpDate>, ServeError> {
                if req.headers().contains_key(IF_UNMODIFIED_SINCE) {
                    Err(ServeError::PreconditionFailed)
                } else {
                    Ok(None)
                }
            }

            return precondition_check(req);
        }
    };

    let if_unmodified_since = header_value_to_http_date(req.headers().get(IF_UNMODIFIED_SINCE));
    let if_modified_since = header_value_to_http_date(req.headers().get(IF_MODIFIED_SINCE));

    if let Some(ref time) = if_unmodified_since {
        if time < &modified_time {
            return Err(ServeError::PreconditionFailed);
        }
    }

    if let Some(ref time) = if_modified_since {
        if time >= &modified_time {
            return Err(ServeError::NotModified);
        }
    }

    Ok(Some(modified_time))
}

fn header_value_to_http_date(header: Option<&HeaderValue>) -> Option<HttpDate> {
    header.and_then(|v| {
        std::str::from_utf8(v.as_ref())
            .ok()
            .map(<HttpDate as FromStr>::from_str)
            .and_then(Result::ok)
    })
}

pub(super) fn date_to_bytes(date: HttpDate) -> Bytes {
    struct BytesMutWriter<'a>(&'a mut BytesMut);

    impl Write for BytesMutWriter<'_> {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            self.0.extend_from_slice(s.as_bytes());
            Ok(())
        }
    }

    let mut bytes = BytesMut::with_capacity(29);
    write!(&mut BytesMutWriter(&mut bytes), "{date}").unwrap();
    bytes.freeze()
}
