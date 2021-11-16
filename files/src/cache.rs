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
                let accept_encoding = encoding
                    .to_str()
                    .unwrap()
                    .replace(' ', "")
                    .split(',')
                    .filter_map(AcceptEncoding::new)
                    .filter(|enc| self.encodings.contains(enc.encoding()))
                    .next();

                match accept_encoding {
                    Some(accept_encoding) => {
                        let enc = accept_encoding.encoding();
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
                        Ok(Some(file.into_response(req)))
                    }
                    None => Ok(None),
                }
            }
            None => Ok(None),
        }
    }
}
