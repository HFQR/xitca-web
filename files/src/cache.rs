pub use http_encoding::ContentEncoding;

use std::path::PathBuf;

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
        match req.headers().get(ACCEPT_ENCODING) {
            Some(encoding) => {
                let encoding = ContentEncoding::from(encoding.to_str().unwrap());

                let enc = match self.encodings.iter().filter(|enc| **enc == encoding).next() {
                    Some(enc) => enc.as_path_extension(),
                    None => return Ok(None),
                };

                let mut cache_path = path.clone();

                match cache_path.extension() {
                    Some(ext) => {
                        let mut ext = ext.to_os_string();
                        ext.push(enc);
                        cache_path.set_extension(ext)
                    }
                    None => cache_path.set_extension(enc),
                };

                let file = NamedFile::open(&cache_path).await?;
                Ok(Some(file.into_response(req)))
            }
            None => Ok(None),
        }
    }
}
