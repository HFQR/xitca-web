use crate::{
    client::{Client, ClientBorrowMut},
    error::Error,
    execute::Execute,
};

use super::{Transaction, TransactionOwned};

/// The isolation level of a database transaction.
#[derive(Debug, Copy, Clone)]
#[non_exhaustive]
pub enum IsolationLevel {
    /// Equivalent to `ReadCommitted`.
    ReadUncommitted,
    /// An individual statement in the transaction will see rows committed before it began.
    ReadCommitted,
    /// All statements in the transaction will see the same view of rows committed before the first query in the
    /// transaction.
    RepeatableRead,
    /// The reads and writes in this transaction must be able to be committed as an atomic "unit" with respect to reads
    /// and writes of all other concurrent serializable transactions without interleaving.
    Serializable,
}

impl IsolationLevel {
    const PREFIX: &str = " ISOLATION LEVEL ";
    const READ_UNCOMMITTED: &str = "READ UNCOMMITTED,";
    const READ_COMMITTED: &str = "READ COMMITTED,";
    const REPEATABLE_READ: &str = "REPEATABLE READ,";
    const SERIALIZABLE: &str = "SERIALIZABLE,";

    fn write(self, str: &mut String) {
        str.reserve(const { Self::PREFIX.len() + Self::READ_UNCOMMITTED.len() });
        str.push_str(Self::PREFIX);
        str.push_str(match self {
            IsolationLevel::ReadUncommitted => Self::READ_UNCOMMITTED,
            IsolationLevel::ReadCommitted => Self::READ_COMMITTED,
            IsolationLevel::RepeatableRead => Self::REPEATABLE_READ,
            IsolationLevel::Serializable => Self::SERIALIZABLE,
        });
    }
}

/// A builder for database transactions.
pub struct TransactionBuilder {
    isolation_level: Option<IsolationLevel>,
    read_only: Option<bool>,
    deferrable: Option<bool>,
}

impl TransactionBuilder {
    pub const fn new() -> Self {
        Self {
            isolation_level: None,
            read_only: None,
            deferrable: None,
        }
    }

    /// Sets the isolation level of the transaction.
    pub fn isolation_level(mut self, isolation_level: IsolationLevel) -> Self {
        self.isolation_level = Some(isolation_level);
        self
    }

    /// Sets the access mode of the transaction.
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = Some(read_only);
        self
    }

    /// Sets the deferrability of the transaction.
    ///
    /// If the transaction is also serializable and read only, creation of the transaction may block, but when it
    /// completes the transaction is able to run with less overhead and a guarantee that it will not be aborted due to
    /// serialization failure.
    pub fn deferrable(mut self, deferrable: bool) -> Self {
        self.deferrable = Some(deferrable);
        self
    }

    /// Begins the transaction with a borrowned client.
    ///
    /// The transaction will roll back by default - use [`Transaction::commit`] method to commit it.
    pub async fn begin<C>(self, cli: &mut C) -> Result<Transaction<'_, C>, Error>
    where
        C: ClientBorrowMut,
    {
        // marker check to ensure exclusive borrowing Client. see ClientBorrowMut for detail
        self._begin(cli.borrow_cli_mut()).await.map(|_| Transaction::new(cli))
    }

    /// Begins the transaction with an owned client.
    ///
    /// The transaction will roll back by default - use [`Transaction::commit`] method to commit it.
    pub async fn begin_owned<C>(self, mut cli: C) -> Result<TransactionOwned<C>, Error>
    where
        C: ClientBorrowMut,
    {
        // marker check to ensure exclusive borrowing Client. see ClientBorrowMut for detail
        self._begin(cli.borrow_cli_mut())
            .await
            .map(|_| TransactionOwned::new(cli))
    }

    async fn _begin<F>(self, cli: &mut Client) -> Result<u64, Error>
    where
        for<'s, 'c> &'s str: Execute<&'s Client, ExecuteOutput = F>,
        F: Future<Output = Result<u64, Error>> + Send,
    {
        let mut query = String::from("START TRANSACTION");

        let Self {
            isolation_level,
            read_only,
            deferrable,
        } = self;

        if let Some(isolation_level) = isolation_level {
            isolation_level.write(&mut query);
        }

        if let Some(read_only) = read_only {
            let s = if read_only { " READ ONLY," } else { " READ WRITE," };
            query.push_str(s);
        }

        if let Some(deferrable) = deferrable {
            let s = if deferrable { " DEFERRABLE" } else { " NOT DEFERRABLE" };
            query.push_str(s);
        }

        if query.ends_with(',') {
            query.pop();
        }

        query.as_str().execute(cli).await
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use crate::{
        Client, Postgres,
        client::{ClientBorrow, ClientBorrowMut},
    };

    use super::TransactionBuilder;

    #[tokio::test]
    async fn client_borrow_mut() {
        #[derive(Clone)]
        struct PanicCli(Arc<Client>);

        impl PanicCli {
            fn new(cli: Client) -> Self {
                Self(Arc::new(cli))
            }
        }

        impl ClientBorrow for PanicCli {
            fn borrow_cli_ref(&self) -> &Client {
                &self.0
            }
        }

        impl ClientBorrowMut for PanicCli {
            fn borrow_cli_mut(&mut self) -> &mut Client {
                Arc::get_mut(&mut self.0).unwrap()
            }
        }

        let (cli, drv) = Postgres::new("postgres://postgres:postgres@localhost:5432")
            .connect()
            .await
            .unwrap();

        tokio::spawn(drv.into_future());

        let mut cli = PanicCli::new(cli);

        {
            let _tx = TransactionBuilder::new().begin(&mut cli).await.unwrap();
        }

        let res = tokio::spawn(async move {
            let _cli2 = cli.clone();
            let _tx = TransactionBuilder::new().begin_owned(cli).await.unwrap();
        })
        .await
        .err()
        .unwrap();

        assert!(res.is_panic());
    }
}
