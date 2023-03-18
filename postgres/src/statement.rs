//! Statement module is mostly copy/paste from `tokio_postgres::statement`

use alloc::{boxed::Box, vec::Vec};

use postgres_protocol::message::frontend;

use super::{client::Client, column::Column, Type};

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
                let res = self
                    .client
                    .try_encode_with(|b| frontend::close(b'S', &statement.name, b).map(|_| frontend::sync(b)));

                if let Ok(msg) = res {
                    self.client.do_send(msg);
                }
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
    #[inline]
    pub fn params(&self) -> &[Type] {
        &self.params
    }

    /// Returns information about the columns returned when the statement is queried.
    #[inline]
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
