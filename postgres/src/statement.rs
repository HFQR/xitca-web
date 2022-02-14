//! Statement module is mostly copy/paste from `tokio_postgres::statement`

use std::fmt;

use postgres_protocol::message::frontend;
use postgres_types::Type;
use xitca_io::bytes::BytesMut;

use super::Client;

#[derive(Clone)]
pub struct Statement {
    name: String,
    params: Vec<Type>,
    columns: Vec<Column>,
}

impl Drop for Statement {
    fn drop(&mut self) {
        // if !self.client.closed() {
        //     let buf = &mut BytesMut::new();

        //     frontend::close(b'S', &self.name, buf).unwrap();
        //     frontend::sync(buf);

        //     let msg = buf.split().freeze();

        //     // let _ = client.send(msg);
        // }
    }
}

impl Statement {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// Returns the expected types of the statement's parameters.
    pub fn params(&self) -> &[Type] {
        &self.params
    }

    /// Returns information about the columns returned when the statement is queried.
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }
}

/// Information about a column of a query.
#[derive(Clone)]
pub struct Column {
    name: String,
    type_: Type,
}

impl Column {
    pub(crate) fn new(name: String, type_: Type) -> Column {
        Column { name, type_ }
    }

    /// Returns the name of the column.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the type of the column.
    pub fn type_(&self) -> &Type {
        &self.type_
    }
}

impl fmt::Debug for Column {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("Column")
            .field("name", &self.name)
            .field("type", &self.type_)
            .finish()
    }
}
