pub(crate) mod connector;
#[cfg(feature = "dangerous")]
pub(crate) mod dangerous;

pub type TlsStream = Box<dyn xitca_io::io::AsyncIoDyn + Send>;
