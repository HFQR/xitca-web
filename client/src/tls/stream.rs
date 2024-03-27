use xitca_io::io::AsyncIoDyn;

pub type TlsStream = Box<dyn AsyncIoDyn + Send + Sync>;
