//! Statement module is mostly copy/paste from `tokio_postgres::statement`

use std::{fmt, ops::Deref};

use postgres_protocol::message::frontend;
use postgres_types::Type;
use xitca_io::bytes::BytesMut;

use super::Client;

/// Guarded statement that would cancel itself when dropped.
pub struct StatementGuarded<'a> {
    statement: Option<Statement>,
    client: &'a Client,
}

impl Deref for StatementGuarded<'_> {
    type Target = Statement;

    fn deref(&self) -> &Self::Target {
        self.statement.as_ref().unwrap()
    }
}

impl Drop for StatementGuarded<'_> {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl<'a> StatementGuarded<'a> {
    /// Leak the statement and it would not be cancelled for current connection.
    pub fn leak(mut self) -> Statement {
        self.statement.take().unwrap()
    }

    fn cancel(&mut self) {
        if let Some(statement) = self.statement.take() {
            if !self.client.closed() {
                let buf = &mut BytesMut::new();

                frontend::close(b'S', &statement.name, buf).unwrap();
                frontend::sync(buf);

                let msg = buf.split().freeze();

                let _ = self.client.send(msg);
            }
        }
    }
}

#[derive(Clone)]
pub struct Statement {
    name: String,
    params: Vec<Type>,
    columns: Vec<Column>,
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

    /// Convert self to [StatementGuarded] which would cancel the statement on drop.
    pub fn into_guarded(self, client: &Client) -> StatementGuarded<'_> {
        StatementGuarded {
            statement: Some(self),
            client,
        }
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
