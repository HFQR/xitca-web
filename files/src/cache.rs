pub use http_encoding::ContentEncoding;

use std::path::PathBuf;

use http_encoding::AcceptEncoding;
use xitca_http::{
    body::ResponseBody,
    http::{header::ACCEPT_ENCODING, Request, Response},
};

use crate::{error::Error, named::NamedFile};

pub(crate) struct Cacher {
    encodings: Box<[ContentEncoding]>,
}

impl Cacher {
    pub(crate) fn new(encodings: Vec<ContentEncoding>) -> Self {
        Self {
            encodings: encodings.into_boxed_slice(),
        }
    }

    pub(crate) async fn serve_cached<B>(
        &self,
        req: &mut Request<B>,
        path: &PathBuf,
    ) -> Result<Option<Response<ResponseBody>>, Error>
    where
        B: Default,
    {
        match req.headers_mut().remove(ACCEPT_ENCODING) {
            Some(encoding) => {
                let value = encoding.to_str().unwrap();

                for enc in self
                    .encodings
                    .iter()
                    .filter_map(|enc| AcceptEncoding::try_parse(value, *enc).ok())
                {
                    match enc {
                        // TODO: AcceptEncoding::try_parse should not return identity encoding.
                        ContentEncoding::Identity => continue,
                        enc => {
                            let mut cache_path = path.clone();

                            match cache_path.extension() {
                                Some(ext) => {
                                    let mut ext = ext.to_os_string();
                                    ext.push(enc.as_path_extension());
                                    cache_path.set_extension(ext)
                                }
                                None => cache_path.set_extension(enc.as_path_extension()),
                            };

                            let file = NamedFile::open(&cache_path).await?;
                            return Ok(Some(file.into_response(req)));
                        }
                    }
                }

                Ok(None)
            }
            None => Ok(None),
        }
    }
}
