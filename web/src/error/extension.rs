use core::{any::type_name, fmt};

use std::error;

use super::{error_from_service, forward_blank_internal};

/// error type for typed instance can't be found from [`Extensions`]
///
/// [`Extensions`]: crate::http::Extensions
#[derive(Debug)]
pub struct ExtensionNotFound(&'static str);

impl ExtensionNotFound {
    pub(crate) fn from_type<T>() -> Self {
        Self(type_name::<T>())
    }
}

impl fmt::Display for ExtensionNotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} type can't be found from Extensions", self.0)
    }
}

impl error::Error for ExtensionNotFound {}

error_from_service!(ExtensionNotFound);
forward_blank_internal!(ExtensionNotFound);
