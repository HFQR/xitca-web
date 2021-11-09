#![forbid(unsafe_code)]
#![feature(generic_associated_types, type_alias_impl_trait)]
#![allow(dead_code, unused_variables, unused_imports)]

mod chunked;

use std::{
    future::{ready, Future, Ready},
    path::PathBuf,
};

use tracing::error;
use xitca_http::{
    body::ResponseBody,
    http::{Request, Response},
};
use xitca_service::{Service, ServiceFactory};

pub struct Files {
    path: String,
    directory: PathBuf,
    index: Option<String>,
    show_index: bool,
    redirect_to_slash: bool,
    hidden_files: bool,
}

impl Files {
    pub fn new<T: Into<PathBuf>>(mount_path: &str, serve_from: T) -> Self {
        let orig_dir = serve_from.into();
        let directory = orig_dir.canonicalize().unwrap_or_else(|_| {
            error!("Specified path is not a directory: {:?}", orig_dir);
            PathBuf::new()
        });

        Files {
            path: mount_path.trim_end_matches('/').to_owned(),
            directory,
            index: None,
            show_index: false,
            redirect_to_slash: false,
            hidden_files: false,
        }
    }

    /// Show files listing for directories.
    ///
    /// By default show files listing is disabled.
    ///
    /// When used with [`Files::index_file()`], files listing is shown as a fallback
    /// when the index file is not found.
    pub fn show_files_listing(mut self) -> Self {
        self.show_index = true;
        self
    }

    /// Redirects to a slash-ended path when browsing a directory.
    ///
    /// By default never redirect.
    pub fn redirect_to_slash_directory(mut self) -> Self {
        self.redirect_to_slash = true;
        self
    }

    /// Enables serving hidden files and directories, allowing a leading dots in url fragments.
    pub fn use_hidden_files(mut self) -> Self {
        self.hidden_files = true;
        self
    }
}

impl ServiceFactory<Request<()>> for Files {
    type Response = Response<ResponseBody>;
    type Error = ();
    type Config = ();
    type Service = FilesService;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        async { Ok(FilesService) }
    }
}

pub struct FilesService;

impl Service<Request<()>> for FilesService {
    type Response = Response<ResponseBody>;
    type Error = ();
    type Ready<'f> = Ready<Result<(), Self::Error>>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        ready(Ok(()))
    }

    fn call(&self, req: Request<()>) -> Self::Future<'_> {
        async { todo!() }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use xitca_web::{request::WebRequest, App};

    struct Adaptor<F>(F);

    impl<'r, 's, D, F> ServiceFactory<&'r mut WebRequest<'s, D>> for Adaptor<F>
    where
        F: ServiceFactory<Request<()>>,
        F::Service: 'static,
    {
        type Response = F::Response;
        type Error = F::Error;
        type Config = F::Config;
        type Service = AdaptorService<F::Service>;
        type InitError = F::InitError;
        type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

        fn new_service(&self, cfg: Self::Config) -> Self::Future {
            let factory = self.0.new_service(cfg);
            async {
                let service = factory.await?;
                Ok(AdaptorService(service))
            }
        }
    }

    struct AdaptorService<S>(S);

    impl<'r, 's, D, S> Service<&'r mut WebRequest<'s, D>> for AdaptorService<S>
    where
        S: Service<Request<()>> + 'static,
    {
        type Response = S::Response;
        type Error = S::Error;
        type Ready<'f> = S::Ready<'f>;
        type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

        fn ready(&self) -> Self::Ready<'_> {
            self.0.ready()
        }

        fn call(&self, req: &'r mut WebRequest<'s, D>) -> Self::Future<'_> {
            async move {
                let (parts, body) = std::mem::take(req.request_mut()).into_parts();
                let req_new = Request::from_parts(parts, ());
                let req_org = Request::new(body);
                *req.request_mut() = req_org;

                self.0.call(req_new).await
            }
        }
    }

    #[tokio::test]
    async fn app() {
        let app = App::new().service(Adaptor(Files::new("/", "./")));

        let app = app.new_service(()).await.unwrap();
    }
}
