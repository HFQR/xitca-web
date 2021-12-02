use std::{
    future::{ready, Future, Ready},
    mem, path,
};

use tracing::error;
use xitca_http::{
    body::ResponseBody,
    http::{header::LOCATION, HeaderValue, IntoResponse, Request, Response, StatusCode},
};
use xitca_service::{Service, ServiceFactory};

use crate::{
    directory::{Directory, DirectoryRender},
    error::Error,
    named::NamedFile,
    path_buf::PathBuf,
};

pub struct Files<F = DirectoryRender> {
    path: String,
    directory: path::PathBuf,
    index: Option<String>,
    show_index: bool,
    redirect_to_slash: bool,
    hidden_files: bool,
    #[cfg(feature = "cache-compress")]
    cache_encodings: Vec<crate::cache::ContentEncoding>,
    directory_render: F,
}

impl Files<DirectoryRender> {
    pub fn new<T: Into<path::PathBuf>>(mount_path: &str, serve_from: T) -> Files<DirectoryRender> {
        let orig_dir = serve_from.into();
        let directory = orig_dir.canonicalize().unwrap_or_else(|_| {
            error!("Specified path is not a directory: {:?}", orig_dir);
            path::PathBuf::new()
        });

        Files {
            path: mount_path.trim_end_matches('/').to_owned(),
            directory,
            index: None,
            show_index: false,
            redirect_to_slash: false,
            hidden_files: false,
            #[cfg(feature = "cache-compress")]
            cache_encodings: Vec::new(),
            directory_render: DirectoryRender,
        }
    }
}

impl<F> Files<F> {
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

    /// Set index file
    ///
    /// Shows specific index file for directories instead of
    /// showing files listing.
    ///
    /// If the index file is not found, files listing is shown as a fallback if
    /// [`Files::show_files_listing()`] is set.
    pub fn index_file<T: Into<String>>(mut self, index: T) -> Self {
        self.index = Some(index.into());
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

    #[cfg(feature = "cache-compress")]
    /// Insert new cacheable encoding to file service.
    ///
    /// Insert order matters. The encoding inserted later would have lower priority.
    pub fn insert_cache_encoding(mut self, encoding: crate::cache::ContentEncoding) -> Self {
        self.cache_encodings.push(encoding);
        self
    }

    /// Change the directory render of index files list.
    ///
    /// See [DirectoryRender] for example implementation.
    pub fn directory_render<F1, S>(self, directory_render: F1) -> Files<F1>
    where
        F1: for<'d> ServiceFactory<(Request<()>, Directory<'d>), Service = S, Config = ()>,
        S: for<'d> Service<(Request<()>, Directory<'d>), Response = Response<ResponseBody>, Error = Error>,
    {
        Files {
            path: self.path,
            directory: self.directory,
            index: self.index,
            show_index: self.show_index,
            redirect_to_slash: self.redirect_to_slash,
            hidden_files: self.hidden_files,
            #[cfg(feature = "cache-compress")]
            cache_encodings: self.cache_encodings,
            directory_render,
        }
    }
}

impl<F, S, B> ServiceFactory<&mut Request<B>> for Files<F>
where
    // TODO: This is a limitation of HRTB lang feature. It can not bound to generic B type and lead to compile error.
    // The alternative for now is to drop request body and use concrete type () for DirectoryRender service.
    F: for<'d> ServiceFactory<(Request<()>, Directory<'d>), Service = S, Config = ()>,
    S: for<'d> Service<(Request<()>, Directory<'d>), Response = Response<ResponseBody>, Error = Error>,
    B: Default,
{
    type Response = Response<ResponseBody>;
    type Error = Error;
    type Config = ();
    type Service = FilesService<S>;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        let directory_render = self.directory_render.new_service(());
        let directory = self.directory.clone();
        let index = self.index.clone();
        let show_index = self.show_index;
        let redirect_to_slash = self.redirect_to_slash;
        let hidden_files = self.hidden_files;

        #[cfg(feature = "cache-compress")]
        let cacher = crate::cache::Cacher::new(self.cache_encodings.clone());

        async move {
            let directory_render = directory_render.await.ok().unwrap();
            Ok(FilesService {
                directory,
                index,
                show_index,
                redirect_to_slash,
                hidden_files,
                #[cfg(feature = "cache-compress")]
                cacher,
                directory_render,
            })
        }
    }
}

pub struct FilesService<S> {
    directory: path::PathBuf,
    index: Option<String>,
    show_index: bool,
    redirect_to_slash: bool,
    hidden_files: bool,
    #[cfg(feature = "cache-compress")]
    cacher: crate::cache::Cacher,
    directory_render: S,
}

impl<'r, S, B> Service<&'r mut Request<B>> for FilesService<S>
where
    S: for<'d> Service<(Request<()>, Directory<'d>), Response = Response<ResponseBody>, Error = Error>,
    B: Default,
{
    type Response = Response<ResponseBody>;
    type Error = Error;
    type Ready<'f>
    where
        S: 'f,
    = Ready<Result<(), Self::Error>>;
    type Future<'f>
    where
        S: 'f,
    = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        ready(Ok(()))
    }

    fn call(&self, req: &'r mut Request<B>) -> Self::Future<'_> {
        async move {
            let real_path = PathBuf::parse_path(req.uri().path(), self.hidden_files)?;
            let path = self.directory.join(&real_path);
            path.canonicalize()?;

            let is_dir = path.is_dir();
            let redirect = self.redirect_to_slash && !req.uri().path().ends_with('/');
            let show_index = self.show_index;

            match (is_dir, redirect, show_index, self.index.as_ref()) {
                (false, _, _, _) => {
                    #[cfg(feature = "cache-compress")]
                    {
                        if let Ok(Some(res)) = self.cacher.serve_cached(req, &path).await {
                            return Ok(res);
                        }
                    }

                    let file = NamedFile::open(&path).await?;
                    Ok(file.into_response(req))
                }
                (_, _, false, None) => Err(Error::IsDirectory),
                (true, true, _, _) => {
                    let redirect_to = format!("{}/", req.uri().path());

                    let mut res = req.as_response(ResponseBody::None);
                    *res.status_mut() = StatusCode::FOUND;
                    res.headers_mut()
                        .append(LOCATION, HeaderValue::try_from(redirect_to).unwrap());

                    Ok(res)
                }
                (_, _, _, Some(index)) => {
                    let named_path = path.join(index);
                    match NamedFile::open(named_path).await {
                        Ok(file) => Ok(file.into_response(req)),
                        Err(_) if show_index => {
                            self.directory_render.ready().await?;
                            let dir = Directory::new(&self.directory, &path);
                            let req = mem::take(req).map(|_| ());
                            self.directory_render.call((req, dir)).await
                        }
                        Err(e) => Err(e.into()),
                    }
                }
                (_, _, true, _) => {
                    self.directory_render.ready().await?;
                    let dir = Directory::new(&self.directory, &path);
                    let req = mem::take(req).map(|_| ());
                    self.directory_render.call((req, dir)).await
                }
            }
        }
    }
}
