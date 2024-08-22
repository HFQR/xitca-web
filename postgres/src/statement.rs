//! Statement module is mostly copy/paste from `tokio_postgres::statement`

use core::ops::Deref;

use postgres_protocol::message::frontend;

use super::{client::Client, column::Column, Type};

/// Guarded statement that would cancel itself when dropped.
pub struct StatementGuarded<C>
where
    C: Deref<Target = Client>,
{
    statement: Option<Statement>,
    client: C,
}

impl<C> AsRef<Statement> for StatementGuarded<C>
where
    C: Deref<Target = Client>,
{
    fn as_ref(&self) -> &Statement {
        self.statement.as_ref().unwrap()
    }
}

impl<C> Drop for StatementGuarded<C>
where
    C: Deref<Target = Client>,
{
    fn drop(&mut self) {
        self.cancel();
    }
}

impl<C> StatementGuarded<C>
where
    C: Deref<Target = Client>,
{
    /// Leak the statement and it would not be cancelled for current connection.
    pub fn leak(mut self) -> Statement {
        self.statement.take().unwrap()
    }

    fn cancel(&mut self) {
        if let Some(statement) = self.statement.take() {
            let _ = self.client.tx.send_with(|buf| {
                frontend::close(b'S', &statement.name, buf)?;
                frontend::sync(buf);
                Ok(())
            });
        }
    }
}

#[derive(Clone, Default)]
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

    pub(crate) fn params_assert(&self, params: &impl ExactSizeIterator) {
        assert_eq!(
            self.params().len(),
            params.len(),
            "expected {} parameters but got {}",
            self.params().len(),
            params.len()
        );
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
    pub fn into_guarded<C>(self, client: C) -> StatementGuarded<C>
    where
        C: Deref<Target = Client>,
    {
        StatementGuarded {
            statement: Some(self),
            client,
        }
    }
}
