use http::{
    Request,
    header::{HeaderValue, IF_MODIFIED_SINCE, IF_UNMODIFIED_SINCE},
};

use super::{
    buf::buf_write_header,
    error::ServeError,
    http_date::{HttpDate, IMF_FIXDATE_LENGTH},
    runtime::Meta,
};

pub(super) fn mod_date_check<Ext, M>(req: &Request<Ext>, meta: &mut M) -> Result<Option<HttpDate>, ServeError>
where
    M: Meta,
{
    let mod_date = match meta.modified() {
        Some(modified) => HttpDate::from(modified),
        None => {
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

    if let Some(ref date) = to_http_date(req.headers().get(IF_UNMODIFIED_SINCE)) {
        if date < &mod_date {
            return Err(ServeError::PreconditionFailed);
        }
    }

    if let Some(ref date) = to_http_date(req.headers().get(IF_MODIFIED_SINCE)) {
        if date >= &mod_date {
            return Err(ServeError::NotModified);
        }
    }

    Ok(Some(mod_date))
}

fn to_http_date(header: Option<&HeaderValue>) -> Option<HttpDate> {
    HttpDate::parse(header?.to_str().ok()?)
}

pub(super) fn date_to_header(date: HttpDate) -> HeaderValue {
    buf_write_header!(IMF_FIXDATE_LENGTH, "{date}")
}
