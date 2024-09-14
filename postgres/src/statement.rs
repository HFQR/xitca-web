//! Statement module is mostly copy/paste from `tokio_postgres::statement`

use core::ops::Deref;

use super::{column::Column, types::Type};

/// trait for generic over the ability to cancel a statement.
///
/// when using new type with [StatementGuarded] the client type has to impl this trait
/// to properly cancel a statement on drop of guard.
pub trait CancelStatement {
    fn cancel(&self, stmt: &Statement);
}

/// Guarded statement that would cancel itself when dropped.
pub struct StatementGuarded<C>
where
    C: CancelStatement,
{
    stmt: Option<Statement>,
    cli: C,
}

impl<C> AsRef<Statement> for StatementGuarded<C>
where
    C: CancelStatement,
{
    #[inline]
    fn as_ref(&self) -> &Statement {
        self
    }
}

impl<C> Deref for StatementGuarded<C>
where
    C: CancelStatement,
{
    type Target = Statement;

    fn deref(&self) -> &Self::Target {
        self.stmt.as_ref().unwrap()
    }
}

impl<C> Drop for StatementGuarded<C>
where
    C: CancelStatement,
{
    fn drop(&mut self) {
        if let Some(stmt) = self.stmt.take() {
            self.cli.cancel(&stmt);
        }
    }
}

impl<C> StatementGuarded<C>
where
    C: CancelStatement,
{
    /// Leak the statement and it would not be cancelled for current connection.
    pub fn leak(mut self) -> Statement {
        self.stmt.take().unwrap()
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
        C: CancelStatement,
    {
        StatementGuarded { stmt: Some(self), cli }
    }
}

#[cfg(feature = "compat")]
pub(crate) mod compat {
    use std::sync::Arc;

    use super::{CancelStatement, Statement};

    #[derive(Clone)]
    pub struct StatementGuarded<C>
    where
        C: CancelStatement,
    {
        inner: Arc<_StatementGuarded<C>>,
    }

    struct _StatementGuarded<C>
    where
        C: CancelStatement,
    {
        stmt: Statement,
        cli: C,
    }

    impl<C> Drop for _StatementGuarded<C>
    where
        C: CancelStatement,
    {
        fn drop(&mut self) {
            self.cli.cancel(&self.stmt)
        }
    }

    impl<C> AsRef<Statement> for StatementGuarded<C>
    where
        C: CancelStatement,
    {
        fn as_ref(&self) -> &Statement {
            &self.inner.stmt
        }
    }

    impl<C> StatementGuarded<C>
    where
        C: CancelStatement,
    {
        pub fn new(stmt: Statement, cli: C) -> Self {
            Self {
                inner: Arc::new(_StatementGuarded { stmt, cli }),
            }
        }
    }
}
