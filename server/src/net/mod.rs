use std::{io, sync::Arc};

use xitca_io::net::{Stream, TcpListener};

#[cfg(unix)]
use xitca_io::net::UnixListener;

use tracing::info;
#[cfg(feature = "quic")]
use xitca_io::net::{QuicListener, QuicListenerBuilder};

/// trait for defining how socket listener would accept remote connection and omit connection stream asynchronously
///
/// listener must be thread safe type for parallel accessing by multiple worker threads.
///
/// # Examples
/// ```rust
/// use std::io;
///
/// use xitca_io::net::Stream;
/// use xitca_server::net::{IntoListener, Listen};
/// use xitca_service::fn_service;
///
/// // arbitrary socket type
/// struct MySocket;
///
/// impl Listen for MySocket {
///     async fn accept(&self) -> io::Result<Stream> {
///         todo!("defining how my socket would accept remote connection in the type of Stream")
///     }
/// }
///
/// // arbitrary builder type for socket. allow for additional logic when constructing socket type
/// struct MySocketBuilder;
///
/// impl IntoListener for MySocketBuilder {
///     type Listener = MySocket;
///
///     fn into_listener(self) -> io::Result<Self::Listener> {
///         // transform socket builder to the socket runner type.
///         // this function is called from inside xitca-server and it's possible to tap into it's internal from here.
///         // e.g: accessing the thread local storage or the async runtime(tokio)'s context.
///         Ok(MySocket)
///     }
/// }
///
/// // service function receive connection stream from MySocket's Listen::accept method
/// let service = fn_service(async |stream: Stream| {
///     Ok::<_, io::Error>(())
/// });
///
/// // start a server with socket builder where My socket would be instantiated and it's accepting logic would start and
/// // run the service function when successfully accepted remote connection.
/// let _ = xitca_server::Builder::new().listen("my_socket_service", MySocketBuilder, service);
/// ```
pub trait Listen: Send + Sync {
    fn accept(&self) -> impl Future<Output = io::Result<Stream>> + Send;
}

mod _seal {
    use core::{future::Future, pin::Pin};

    use super::*;

    type BoxFuture<'f, T> = Pin<Box<dyn Future<Output = T> + Send + 'f>>;

    #[doc(hidden)]
    /// dynamic compat trait for [Listen]
    pub trait ListenDyn: Send + Sync {
        fn accept_dyn(&self) -> BoxFuture<io::Result<Stream>>;
    }

    impl<S> ListenDyn for S
    where
        S: Listen,
    {
        #[inline]
        fn accept_dyn(&self) -> BoxFuture<io::Result<Stream>> {
            Box::pin(Listen::accept(self))
        }
    }
}

pub(crate) type ListenerDyn = Arc<dyn _seal::ListenDyn>;

impl Listen for TcpListener {
    async fn accept(&self) -> io::Result<Stream> {
        let (stream, addr) = self.accept().await?;
        let stream = stream.into_std()?;
        Ok(Stream::Tcp(stream, addr))
    }
}

#[cfg(unix)]
impl Listen for UnixListener {
    async fn accept(&self) -> io::Result<Stream> {
        let (stream, _) = self.accept().await?;
        let stream = stream.into_std()?;
        let addr = stream.peer_addr()?;
        Ok(Stream::Unix(stream, addr))
    }
}

#[cfg(feature = "quic")]
impl Listen for QuicListener {
    async fn accept(&self) -> io::Result<Stream> {
        let stream = self.accept().await?;
        let addr = stream.peer_addr();
        Ok(Stream::Udp(stream, addr))
    }
}

/// Helper trait for converting listener types and register them to xitca-server
/// By delay the conversion and make the process happen in server thread(s) it avoid possible panic due to runtime locality.
///
/// This trait is often utilized together with [Listen] trait. Please reference it's doc for examples.
pub trait IntoListener: Send {
    type Listener: Listen;

    fn into_listener(self) -> io::Result<Self::Listener>;
}

impl IntoListener for std::net::TcpListener {
    type Listener = TcpListener;

    fn into_listener(self) -> io::Result<Self::Listener> {
        self.set_nonblocking(true)?;
        let listener = TcpListener::from_std(self)?;
        info!("Started Tcp listening on: {:?}", listener.local_addr().ok());
        Ok(listener)
    }
}

#[cfg(unix)]
impl IntoListener for std::os::unix::net::UnixListener {
    type Listener = UnixListener;

    fn into_listener(self) -> io::Result<Self::Listener> {
        self.set_nonblocking(true)?;
        let listener = UnixListener::from_std(self)?;
        info!("Started Unix listening on: {:?}", listener.local_addr().ok());
        Ok(listener)
    }
}

#[cfg(feature = "quic")]
impl IntoListener for QuicListenerBuilder {
    type Listener = QuicListener;

    fn into_listener(self) -> io::Result<Self::Listener> {
        let udp = self.build()?;
        info!("Started Udp listening on: {:?}", udp.endpoint().local_addr().ok());
        Ok(udp)
    }
}
