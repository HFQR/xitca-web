use core::fmt;

use super::types::Type;

/// Information about a column of a query.
#[derive(Clone)]
pub struct Column {
    name: Box<str>,
    r#type: Type,
}

impl Column {
    pub(crate) fn new(name: &str, r#type: Type) -> Column {
        Column {
            name: Box::from(name),
            r#type,
        }
    }

    /// Returns the name of the column.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the type of the column.
    #[inline]
    pub fn r#type(&self) -> &Type {
        &self.r#type
    }
}

impl fmt::Debug for Column {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("Column")
            .field("name", &self.name)
            .field("type", &self.r#type)
            .finish()
    }
}
