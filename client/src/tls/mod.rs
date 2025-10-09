pub(crate) mod connector;

pub type TlsStream = Box<dyn xitca_io::io::AsyncIoDyn + Send + Sync>;
