pub mod body;
pub mod extension;
pub mod header;
pub mod html;
pub mod path;
pub mod redirect;
pub mod state;
pub mod string;
pub mod uri;

#[cfg(feature = "params")]
pub mod params;

#[cfg(feature = "urlencoded")]
pub mod query;

#[cfg(feature = "urlencoded")]
pub mod form;

#[cfg(feature = "json")]
pub mod json;

#[cfg(feature = "cookie")]
pub mod cookie;

#[cfg(feature = "multipart")]
pub mod multipart;

#[cfg(feature = "websocket")]
pub mod websocket;
