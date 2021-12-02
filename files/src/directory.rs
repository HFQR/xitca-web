use std::{
    fmt::Write,
    fs::DirEntry,
    future::{ready, Future, Ready},
    io,
    path::Path,
};

use askama_escape::{escape as escape_html_entity, Html};
use percent_encoding::{utf8_percent_encode, CONTROLS};
use xitca_http::{
    body::ResponseBody,
    bytes::Bytes,
    http::{
        header::{HeaderValue, CONTENT_TYPE},
        IntoResponse, Request, Response,
    },
};
use xitca_service::{Service, ServiceFactory};

use crate::error::Error;

/// A directory; responds with the generated directory listing.
#[derive(Debug)]
pub struct Directory<'a> {
    /// Base directory.
    pub base: &'a Path,

    /// Path of subdirectory to generate listing for.
    pub path: &'a Path,
}

impl<'a> Directory<'a> {
    /// Create a new directory
    pub fn new(base: &'a Path, path: &'a Path) -> Self {
        Self { base, path }
    }

    /// Is this entry visible from this directory?
    pub fn is_visible(&self, entry: &io::Result<DirEntry>) -> bool {
        if let Ok(ref entry) = *entry {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with('.') {
                    return false;
                }
            }
            if let Ok(ref md) = entry.metadata() {
                let ft = md.file_type();
                return ft.is_dir() || ft.is_file() || ft.is_symlink();
            }
        }
        false
    }
}

// show file url as relative to static path
macro_rules! encode_file_url {
    ($path:ident) => {
        utf8_percent_encode(&$path, CONTROLS)
    };
}

// " -- &quot;  & -- &amp;  ' -- &#x27;  < -- &lt;  > -- &gt;  / -- &#x2f;
macro_rules! encode_file_name {
    ($entry:ident) => {
        escape_html_entity(&$entry.file_name().to_string_lossy(), Html)
    };
}

/// Default Directory Renderer for index files list.
pub struct DirectoryRender;

impl<B> ServiceFactory<(Request<B>, Directory<'_>)> for DirectoryRender {
    type Response = Response<ResponseBody>;
    type Error = Error;
    type Config = ();
    type Service = Self;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        ready(Ok(Self))
    }
}

impl<'d, B> Service<(Request<B>, Directory<'d>)> for DirectoryRender {
    type Response = Response<ResponseBody>;
    type Error = Error;
    type Ready<'f> = Ready<Result<(), Self::Error>>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        ready(Ok(()))
    }

    fn call(&self, (req, dir): (Request<B>, Directory<'d>)) -> Self::Future<'_> {
        async move {
            let index_of = format!("Index of {}", req.uri().path());
            let mut body = String::new();
            let base = Path::new(req.uri().path());

            for entry in dir.path.read_dir()? {
                if dir.is_visible(&entry) {
                    let entry = entry.unwrap();
                    let p = match entry.path().strip_prefix(&dir.path) {
                        Ok(p) if cfg!(windows) => base.join(p).to_string_lossy().replace("\\", "/"),
                        Ok(p) => base.join(p).to_string_lossy().into_owned(),
                        Err(_) => continue,
                    };

                    // if file is a directory, add '/' to the end of the name
                    if let Ok(metadata) = entry.metadata() {
                        if metadata.is_dir() {
                            let _ = write!(
                                body,
                                "<li><a href=\"{}\">{}/</a></li>",
                                encode_file_url!(p),
                                encode_file_name!(entry),
                            );
                        } else {
                            let _ = write!(
                                body,
                                "<li><a href=\"{}\">{}</a></li>",
                                encode_file_url!(p),
                                encode_file_name!(entry),
                            );
                        }
                    } else {
                        continue;
                    }
                }
            }

            let html = format!(
                "<html>\
                 <head><title>{}</title></head>\
                 <body>\
                 <h1>{}</h1>\
                 <ul>{}</ul>\
                 </body>\n\
                 </html>",
                index_of, index_of, body
            );

            let mut res = req.into_response(Bytes::from(html));
            res.headers_mut()
                .append(CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));

            Ok(res)
        }
    }
}
