mod redirect;

#[cfg(feature = "compress")]
mod decompress;

#[cfg(feature = "compress")]
pub use decompress::Decompress;

pub use redirect::FollowRedirect;
