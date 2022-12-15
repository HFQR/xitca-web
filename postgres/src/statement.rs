//! Statement module is mostly copy/paste from `tokio_postgres::statement`

use std::fmt;

use postgres_protocol::message::frontend;
use postgres_types::Type;
use xitca_io::bytes::BytesMut;

use super::Client;

/// Guarded statement that would cancel itself when dropped.
pub struct StatementGuarded<'a> {
    statement: Option<Statement>,
    client: &'a Client,
}

impl AsRef<Statement> for StatementGuarded<'_> {
    fn as_ref(&self) -> &Statement {
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
                let mut buf = BytesMut::new();
                frontend::close(b'S', &statement.name, &mut buf).unwrap();
                frontend::sync(&mut buf);

                // TODO: fix this send. right now it's lazy and do nothing.
                let _f = self.client.send(buf);
            }
        }
    }
}

#[derive(Clone)]
pub struct Statement {
    name: Box<str>,
    params: Box<[Type]>,
    columns: Box<[Column]>,
}

impl Statement {
    pub(crate) fn new(name: String, params: Vec<Type>, columns: Vec<Column>) -> Self {
        Self {
            name: name.into_boxed_str(),
            params: params.into_boxed_slice(),
            columns: columns.into_boxed_slice(),
        }
    }

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

    /// Convert self to a drop guarded statement which would cancel on drop.
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
    name: Box<str>,
    type_: Type,
}

impl Column {
    pub(crate) fn new(name: &str, type_: Type) -> Column {
        Column {
            name: Box::from(name),
            type_,
        }
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
