//! Statement module is mostly copy/paste from `tokio_postgres::statement`

use core::ops::Deref;

use super::{column::Column, prepare::Prepare, types::Type};

/// a statement guard contains a prepared postgres statement.
/// the guard can be dereferenced or borrowed as [`Statement`] which can be used for query apis.
///
/// the guard would cancel it's statement when dropped. generic C type must be a client type impl
/// [`Prepare`] trait to instruct the cancellation.
pub struct StatementGuarded<'a, C>
where
    C: Prepare,
{
    stmt: Option<Statement>,
    cli: &'a C,
}

impl<C> AsRef<Statement> for StatementGuarded<'_, C>
where
    C: Prepare,
{
    #[inline]
    fn as_ref(&self) -> &Statement {
        self
    }
}

impl<C> Deref for StatementGuarded<'_, C>
where
    C: Prepare,
{
    type Target = Statement;

    fn deref(&self) -> &Self::Target {
        self.stmt.as_ref().unwrap()
    }
}

impl<C> Drop for StatementGuarded<'_, C>
where
    C: Prepare,
{
    fn drop(&mut self) {
        if let Some(stmt) = self.stmt.take() {
            self.cli._send_encode_statement_cancel(&stmt);
        }
    }
}

impl<C> StatementGuarded<'_, C>
where
    C: Prepare,
{
    /// Leak the statement and it would not be cancelled for current connection.
    /// does not cause memory leak.
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

    pub fn unnamed<'a, C>(cli: &'a C, stmt: &'a str, types: &'a [Type]) -> StatementUnnamed<'a, C> {
        StatementUnnamed { stmt, types, cli }
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
    #[inline]
    pub fn into_guarded<C>(self, cli: &C) -> StatementGuarded<C>
    where
        C: Prepare,
    {
        StatementGuarded { stmt: Some(self), cli }
    }
}

pub struct StatementUnnamed<'a, C> {
    pub(crate) stmt: &'a str,
    pub(crate) types: &'a [Type],
    pub(crate) cli: &'a C,
}

impl<C> Clone for StatementUnnamed<'_, C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<C> Copy for StatementUnnamed<'_, C> {}

#[cfg(feature = "compat")]
pub(crate) mod compat {
    use core::ops::Deref;

    use std::sync::Arc;

    use super::{Prepare, Statement};

    /// functions the same as [`StatementGuarded`]
    ///
    /// instead of work with a reference this guard offers ownership without named lifetime constraint.
    ///
    /// [`StatementGuarded`]: super::StatementGuarded
    #[derive(Clone)]
    pub struct StatementGuarded<C>
    where
        C: Prepare,
    {
        inner: Arc<_StatementGuarded<C>>,
    }

    struct _StatementGuarded<C>
    where
        C: Prepare,
    {
        stmt: Statement,
        cli: C,
    }

    impl<C> Drop for _StatementGuarded<C>
    where
        C: Prepare,
    {
        fn drop(&mut self) {
            self.cli._send_encode_statement_cancel(&self.stmt)
        }
    }

    impl<C> Deref for StatementGuarded<C>
    where
        C: Prepare,
    {
        type Target = Statement;

        fn deref(&self) -> &Self::Target {
            &self.inner.stmt
        }
    }

    impl<C> AsRef<Statement> for StatementGuarded<C>
    where
        C: Prepare,
    {
        fn as_ref(&self) -> &Statement {
            &self.inner.stmt
        }
    }

    impl<C> StatementGuarded<C>
    where
        C: Prepare,
    {
        /// construct a new statement guard with raw statement and client
        pub fn new(stmt: Statement, cli: C) -> Self {
            Self {
                inner: Arc::new(_StatementGuarded { stmt, cli }),
            }
        }
    }
}
