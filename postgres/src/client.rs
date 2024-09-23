use core::future::Future;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use xitca_io::bytes::BytesMut;
use xitca_unsafe_collection::no_hash::NoHashBuilder;

use super::{
    copy::{r#Copy, CopyIn, CopyOut},
    driver::{
        codec::{self, AsParams, Encode, IntoStream, Response},
        DriverTx,
    },
    error::Error,
    prepare::Prepare,
    query::{ExecuteFuture, Query},
    session::Session,
    statement::{Statement, StatementGuarded, StatementUnnamed},
    transaction::Transaction,
    types::{Oid, ToSql, Type},
};

/// a marker trait to confirm a mut reference of Client can be borrowed from self.
///
/// this is necessary for custom [Client] new types who want to utilize [Transaction] and [CopyIn].
/// these types and their functions only work properly when [Client] is exclusively borrowed.
///
/// # Examples
/// ```rust
/// use std::sync::Arc;
///
/// use xitca_postgres::{dev::ClientBorrowMut, Client};
///
/// // a client wrapper use reference counted smart pointer.
/// // it's easy to create multiple instance of &mut SharedClient with help of cloning of smart pointer
/// // and none of them can be used correctly with Transaction nor CopyIn
/// #[derive(Clone)]
/// struct SharedClient(Arc<Client>);
///
/// // client new type has to impl this trait to mark they can truly offer a mutable reference to Client
/// impl ClientBorrowMut for SharedClient {
///     fn _borrow_mut(&mut self) -> &mut Client {
///         panic!("you can't safely implement this trait with SharedClient. and Transaction::new will cause a panic with it")
///     }
/// }
///
/// // another client wrapper without indirect
/// struct ExclusiveClient(Client);
///
/// // trait can be implemented correctly. marking this new type can be accept by Transaction and CopyIn
/// impl ClientBorrowMut for ExclusiveClient {
///     fn _borrow_mut(&mut self) -> &mut Client {
///         &mut self.0
///     }
/// }
/// ```
///
/// [Transaction]: crate::transaction::Transaction
/// [CopyIn]: crate::copy::CopyIn
pub trait ClientBorrowMut {
    fn _borrow_mut(&mut self) -> &mut Client;
}

/// Client is a handler type for [`Driver`]. it interacts with latter using channel and message for IO operation
/// and de/encoding of postgres protocol in byte format.
///
/// Client expose a set of high level API to make the interaction represented in Rust function and types.
///
/// # Lifetime
/// Client and [`Driver`] have a dependent lifetime where either side can trigger the other part to shutdown.
/// From Client side it's in the form of dropping ownership.
/// ## Examples
/// ```
/// # use core::future::IntoFuture;
/// # use xitca_postgres::{error::Error, Config, Postgres};
/// # async fn shut_down(cfg: Config) -> Result<(), Error> {
/// // connect to a database and spawn driver as async task
/// let (cli, drv) = Postgres::new(cfg).connect().await?;
/// let handle = tokio::spawn(drv.into_future());
///
/// // drop client after finished usage
/// drop(cli);
///
/// // client would notify driver to shutdown when it's dropped.
/// // await on the handle would return a Result of the shutdown outcome from driver side.  
/// let _ = handle.await.unwrap();
///
/// # Ok(())
/// # }
/// ```
///
/// [`Driver`]: crate::driver::Driver
pub struct Client {
    pub(crate) tx: DriverTx,
    pub(crate) cache: Box<ClientCache>,
}

pub(crate) struct ClientCache {
    session: Session,
    type_info: Mutex<CachedTypeInfo>,
}

/// A cache of type info and prepared statements for fetching type info
/// (corresponding to the queries in the [prepare](prepare) module).
struct CachedTypeInfo {
    /// A statement for basic information for a type from its
    /// OID. Corresponds to [TYPEINFO_QUERY](prepare::TYPEINFO_QUERY) (or its
    /// fallback).
    typeinfo: Option<Statement>,
    /// A statement for getting information for a composite type from its OID.
    /// Corresponds to [TYPEINFO_QUERY](prepare::TYPEINFO_COMPOSITE_QUERY).
    typeinfo_composite: Option<Statement>,
    /// A statement for getting information for a composite type from its OID.
    /// Corresponds to [TYPEINFO_QUERY](prepare::TYPEINFO_COMPOSITE_QUERY) (or
    /// its fallback).
    typeinfo_enum: Option<Statement>,
    /// Cache of types already looked up.
    types: HashMap<Oid, Type, NoHashBuilder>,
}

impl Client {
    /// Creates a new prepared statement.
    ///
    /// Prepared statements can be executed repeatedly, and may contain query parameters (indicated by `$1`, `$2`, etc),
    /// which are set when executed. Prepared statements can only be used with the connection that created them.
    pub async fn prepare(&self, query: &str, types: &[Type]) -> Result<StatementGuarded<Self>, Error> {
        self._prepare(query, types).await.map(|stmt| stmt.into_guarded(self))
    }

    /// Executes a statement, returning an async stream of the resulting rows.
    ///
    /// A statement may contain parameters, specified by `$n`, where `n` is the index of the parameter of the list
    /// provided, 1-indexed.
    ///
    /// If the same statement will be repeatedly executed (perhaps with different query parameters), consider preparing
    /// the statement up front with [Client::prepare].
    #[inline]
    pub fn query<'a, S>(
        &self,
        stmt: S,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<<S::Output<'_> as IntoStream>::RowStream<'a>, Error>
    where
        S: Encode + 'a,
    {
        self._query(stmt, params)
    }

    /// The maximally flexible version of [`Client::query`].
    ///
    /// A statement may contain parameters, specified by `$n`, where `n` is the index of the parameter of the list
    /// provided, 1-indexed.
    ///
    /// If the same statement will be repeatedly executed (perhaps with different query parameters), consider preparing
    /// the statement up front with [`Client::prepare`].
    #[inline]
    pub fn query_raw<'a, S, I>(&self, stmt: S, params: I) -> Result<<S::Output<'_> as IntoStream>::RowStream<'a>, Error>
    where
        S: Encode + 'a,
        I: AsParams,
    {
        self._query_raw(stmt, params)
    }

    /// Executes a statement, returning the number of rows modified.
    ///
    /// A statement may contain parameters, specified by `$n`, where `n` is the index of the parameter of the list
    /// provided, 1-indexed.
    ///
    /// If the same statement will be repeatedly executed (perhaps with different query parameters), consider preparing
    /// the statement up front with [Client::prepare].
    ///
    /// If the statement does not modify any rows (e.g. `SELECT`), 0 is returned.
    #[inline]
    pub fn execute<S>(&self, stmt: S, params: &[&(dyn ToSql + Sync)]) -> ExecuteFuture
    where
        S: Encode,
    {
        self._execute(stmt, params)
    }

    /// The maximally flexible version of [`Client::execute`].
    ///
    /// A statement may contain parameters, specified by `$n`, where `n` is the index of the parameter of the list
    /// provided, 1-indexed.
    ///
    /// If the same statement will be repeatedly executed (perhaps with different query parameters), consider preparing
    /// the statement up front with [Client::prepare].
    #[inline]
    pub fn execute_raw<S, I>(&self, stmt: S, params: I) -> ExecuteFuture
    where
        S: Encode,
        I: AsParams,
    {
        self._execute_raw(stmt, params)
    }

    /// Executes a sequence of SQL statements using the simple query protocol, returning async stream of rows.
    ///
    /// Statements should be separated by semicolons. If an error occurs, execution of the sequence will stop at that
    /// point. The simple query protocol returns the values in rows as strings rather than in their binary encodings,
    /// so the associated row type doesn't work with the `FromSql` trait. Rather than simply returning a list of the
    /// rows, this method returns a list of an enum which indicates either the completion of one of the commands,
    /// or a row of data. This preserves the framing between the separate statements in the request.
    ///
    /// # Warning
    ///
    /// Prepared statements should be use for any query which contains user-specified data, as they provided the
    /// functionality to safely embed that data in the request. Do not form statements via string concatenation and pass
    /// them to this method!
    #[inline]
    pub fn query_simple(
        &self,
        stmt: &str,
    ) -> Result<<<&str as Encode>::Output<'_> as IntoStream>::RowStream<'_>, Error> {
        self.query_raw::<_, crate::ZeroParam>(stmt, [])
    }

    #[inline]
    pub fn execute_simple(&self, stmt: &str) -> ExecuteFuture {
        self.execute_raw::<_, crate::ZeroParam>(stmt, [])
    }

    /// Embed prepare statement to the query request itself. Meaning query would finish in one round trip to database.
    /// However it should also be noted that the client type must be referenced during the whole progress and associated
    /// client must be kept around util streaming is finished.
    #[inline]
    pub fn query_unnamed<'a>(
        &'a self,
        stmt: &'a str,
        types: &'a [Type],
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<<<StatementUnnamed<'a, Self> as Encode>::Output<'a> as IntoStream>::RowStream<'a>, Error> {
        self.query(Statement::unnamed(self, stmt, types), params)
    }

    /// start a transaction
    #[inline]
    pub fn transaction(&mut self) -> impl Future<Output = Result<Transaction<Self>, Error>> + Send {
        Transaction::<Self>::builder().begin(self)
    }

    /// Executes a `COPY FROM STDIN` statement, returning a sink used to write the copy data.
    ///
    /// PostgreSQL does not support parameters in `COPY` statements, so this method does not take any. The copy *must*
    /// be explicitly completed via [`CopyIn::finish`]. If it is not, the copy will be aborted.
    #[inline]
    pub fn copy_in(&mut self, stmt: &Statement) -> impl Future<Output = Result<CopyIn<Self>, Error>> + Send {
        CopyIn::new(self, stmt)
    }

    /// Executes a `COPY TO STDOUT` statement, returning async stream of the resulting data.
    ///
    /// PostgreSQL does not support parameters in `COPY` statements, so this method does not take any.
    #[inline]
    pub async fn copy_out(&self, stmt: &Statement) -> Result<CopyOut, Error> {
        CopyOut::new(self, stmt).await
    }

    /// Constructs a cancellation token that can later be used to request cancellation of a query running on the
    /// connection associated with this client.
    pub fn cancel_token(&self) -> Session {
        Session::clone(&self.cache.session)
    }

    /// a lossy hint of running state of io driver. an io driver shutdown can happen
    /// at the same time this api is called.
    pub fn closed(&self) -> bool {
        self.tx.is_closed()
    }

    pub fn typeinfo(&self) -> Option<Statement> {
        self.cache
            .type_info
            .lock()
            .unwrap()
            .typeinfo
            .as_ref()
            .map(Statement::duplicate)
    }

    pub fn set_typeinfo(&self, statement: &Statement) {
        self.cache.type_info.lock().unwrap().typeinfo = Some(statement.duplicate());
    }

    pub fn typeinfo_composite(&self) -> Option<Statement> {
        self.cache
            .type_info
            .lock()
            .unwrap()
            .typeinfo_composite
            .as_ref()
            .map(Statement::duplicate)
    }

    pub fn set_typeinfo_composite(&self, statement: &Statement) {
        self.cache.type_info.lock().unwrap().typeinfo_composite = Some(statement.duplicate());
    }

    pub fn typeinfo_enum(&self) -> Option<Statement> {
        self.cache
            .type_info
            .lock()
            .unwrap()
            .typeinfo_enum
            .as_ref()
            .map(Statement::duplicate)
    }

    pub fn set_typeinfo_enum(&self, statement: &Statement) {
        self.cache.type_info.lock().unwrap().typeinfo_enum = Some(statement.duplicate());
    }

    pub fn type_(&self, oid: Oid) -> Option<Type> {
        self.cache.type_info.lock().unwrap().types.get(&oid).cloned()
    }

    pub fn set_type(&self, oid: Oid, type_: &Type) {
        self.cache.type_info.lock().unwrap().types.insert(oid, type_.clone());
    }

    /// Clears the client's type information cache.
    ///
    /// When user-defined types are used in a query, the client loads their definitions from the database and caches
    /// them for the lifetime of the client. If those definitions are changed in the database, this method can be used
    /// to flush the local cache and allow the new, updated definitions to be loaded.
    pub fn clear_type_cache(&self) {
        self.cache.type_info.lock().unwrap().types.clear();
    }

    pub(crate) fn new(tx: DriverTx, session: Session) -> Self {
        Self {
            tx,
            cache: Box::new(ClientCache {
                session,
                type_info: Mutex::new(CachedTypeInfo {
                    typeinfo: None,
                    typeinfo_composite: None,
                    typeinfo_enum: None,
                    types: HashMap::default(),
                }),
            }),
        }
    }
}

impl ClientBorrowMut for Client {
    fn _borrow_mut(&mut self) -> &mut Client {
        self
    }
}

impl Prepare for Arc<Client> {
    fn _get_type(&self, oid: Oid) -> crate::BoxedFuture<'_, Result<Type, Error>> {
        Client::_get_type(self, oid)
    }
}

impl Query for Arc<Client> {
    #[inline]
    fn _send_encode_query<S, I>(&self, stmt: S, params: I) -> Result<(S::Output<'_>, Response), Error>
    where
        S: Encode,
        I: AsParams,
    {
        Client::_send_encode_query(self, stmt, params)
    }
}

impl Query for Client {
    #[inline]
    fn _send_encode_query<S, I>(&self, stmt: S, params: I) -> Result<(S::Output<'_>, Response), Error>
    where
        S: Encode,
        I: AsParams,
    {
        codec::send_encode_query(&self.tx, stmt, params)
    }
}

impl r#Copy for Client {
    #[inline]
    fn send_one_way<F>(&self, func: F) -> Result<(), Error>
    where
        F: FnOnce(&mut BytesMut) -> Result<(), Error>,
    {
        self.tx.send_one_way(func)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // convert leaked statements to guarded statements.
        // this is to cancel the statement on client go away.
        let (type_info, typeinfo_composite, typeinfo_enum) = {
            let cache = self.cache.type_info.get_mut().unwrap();
            (
                cache.typeinfo.take(),
                cache.typeinfo_composite.take(),
                cache.typeinfo_enum.take(),
            )
        };

        if let Some(stmt) = type_info {
            drop(stmt.into_guarded(&*self));
        }

        if let Some(stmt) = typeinfo_composite {
            drop(stmt.into_guarded(&*self));
        }

        if let Some(stmt) = typeinfo_enum {
            drop(stmt.into_guarded(&*self));
        }
    }
}
