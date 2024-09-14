//! Statement module is mostly copy/paste from `tokio_postgres::statement`

use core::ops::Deref;

use postgres_protocol::message::frontend;

use super::{client::Client, column::Column, types::Type};

/// Guarded statement that would cancel itself when dropped.
pub struct StatementGuarded<C>
where
    C: Deref<Target = Client>,
{
    stmt: Option<Statement>,
    cli: C,
}

impl<C> AsRef<Statement> for StatementGuarded<C>
where
    C: Deref<Target = Client>,
{
    fn as_ref(&self) -> &Statement {
        self.stmt.as_ref().unwrap()
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
        self.stmt.take().unwrap()
    }

    fn cancel(&mut self) {
        if let Some(stmt) = self.stmt.take() {
            cancel(&self.cli, &stmt);
        }
    }
}

fn cancel(cli: &Client, stmt: &Statement) {
    let _ = cli.tx.send(|buf| {
        frontend::close(b'S', &stmt.name, buf)?;
        frontend::sync(buf);
        Ok(())
    });
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
    pub fn into_guarded<C>(self, cli: C) -> StatementGuarded<C>
    where
        C: Deref<Target = Client>,
    {
        StatementGuarded { stmt: Some(self), cli }
    }
}

#[cfg(feature = "compat")]
pub(crate) mod compat {
    use core::ops::Deref;

    use std::sync::Arc;

    use crate::client::Client;

    use super::Statement;

    #[derive(Clone)]
    pub struct StatementGuarded<C>
    where
        C: Deref<Target = Client>,
    {
        inner: Arc<_StatementGuarded<C>>,
    }

    struct _StatementGuarded<C>
    where
        C: Deref<Target = Client>,
    {
        stmt: Statement,
        cli: C,
    }

    impl<C> Drop for _StatementGuarded<C>
    where
        C: Deref<Target = Client>,
    {
        fn drop(&mut self) {
            super::cancel(&self.cli, &self.stmt);
        }
    }

    impl<C> AsRef<Statement> for StatementGuarded<C>
    where
        C: Deref<Target = Client>,
    {
        fn as_ref(&self) -> &Statement {
            &self.inner.stmt
        }
    }

    impl<C> StatementGuarded<C>
    where
        C: Deref<Target = Client>,
    {
        pub fn new(stmt: Statement, cli: C) -> Self {
            Self {
                inner: Arc::new(_StatementGuarded { stmt, cli }),
            }
        }
    }
}
