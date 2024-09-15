use core::future::Future;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use postgres_types::{Oid, Type};
use xitca_io::bytes::BytesMut;
use xitca_unsafe_collection::no_hash::NoHashBuilder;

use super::{
    copy::{r#Copy, CopyIn, CopyOut},
    driver::{
        codec::{self, AsParams, Response},
        DriverTx,
    },
    error::Error,
    iter::slice_iter,
    portal::PortalTrait,
    prepare::Prepare,
    query::{Query, QuerySimple, RowSimpleStream, RowStream},
    session::Session,
    statement::{Statement, StatementGuarded},
    transaction::Transaction,
    types::ToSql,
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
    pub fn query<'a>(&self, stmt: &'a Statement, params: &[&(dyn ToSql + Sync)]) -> Result<RowStream<'a>, Error> {
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
    pub fn query_raw<'a, I>(&self, stmt: &'a Statement, params: I) -> Result<RowStream<'a>, Error>
    where
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
    pub fn execute(
        &self,
        stmt: &Statement,
        params: &[&(dyn ToSql + Sync)],
    ) -> impl Future<Output = Result<u64, Error>> + Send {
        // TODO: call execute_raw when Rust2024 edition capture rule is stabled.
        let res = self._send_encode_query(stmt, slice_iter(params));
        async { res?.try_into_row_affected().await }
    }

    /// The maximally flexible version of [`Client::execute`].
    ///
    /// A statement may contain parameters, specified by `$n`, where `n` is the index of the parameter of the list
    /// provided, 1-indexed.
    ///
    /// If the same statement will be repeatedly executed (perhaps with different query parameters), consider preparing
    /// the statement up front with [Client::prepare].
    #[inline]
    pub fn execute_raw<I>(&self, stmt: &Statement, params: I) -> impl Future<Output = Result<u64, Error>> + Send
    where
        I: AsParams,
    {
        let res = self._send_encode_query(stmt, params);
        async { res?.try_into_row_affected().await }
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
    pub fn query_simple(&self, stmt: &str) -> Result<RowSimpleStream, Error> {
        self._query_simple(stmt)
    }

    #[inline]
    pub fn execute_simple(&self, stmt: &str) -> impl Future<Output = Result<u64, Error>> + Send {
        self._execute_simple(stmt)
    }

    /// start a transaction
    #[inline]
    pub fn transaction(&mut self) -> impl Future<Output = Result<Transaction<Client>, Error>> + Send {
        Transaction::new(self)
    }

    /// Executes a `COPY FROM STDIN` statement, returning a sink used to write the copy data.
    ///
    /// PostgreSQL does not support parameters in `COPY` statements, so this method does not take any. The copy *must*
    /// be explicitly completed via [`CopyIn::finish`]. If it is not, the copy will be aborted.
    #[inline]
    pub fn copy_in(&mut self, stmt: &Statement) -> impl Future<Output = Result<CopyIn<Client>, Error>> + Send {
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
        self.cache.type_info.lock().unwrap().typeinfo.clone()
    }

    pub fn set_typeinfo(&self, statement: &Statement) {
        self.cache.type_info.lock().unwrap().typeinfo = Some(statement.clone());
    }

    pub fn typeinfo_composite(&self) -> Option<Statement> {
        self.cache.type_info.lock().unwrap().typeinfo_composite.clone()
    }

    pub fn set_typeinfo_composite(&self, statement: &Statement) {
        self.cache.type_info.lock().unwrap().typeinfo_composite = Some(statement.clone());
    }

    pub fn typeinfo_enum(&self) -> Option<Statement> {
        self.cache.type_info.lock().unwrap().typeinfo_enum.clone()
    }

    pub fn set_typeinfo_enum(&self, statement: &Statement) {
        self.cache.type_info.lock().unwrap().typeinfo_enum = Some(statement.clone());
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
    #[inline]
    fn _prepare(&self, query: &str, types: &[Type]) -> impl Future<Output = Result<Statement, Error>> + Send {
        Client::_prepare(self, query, types)
    }

    fn _send_encode_statement_cancel(&self, stmt: &Statement) {
        Client::_send_encode_statement_cancel(self, stmt)
    }
}

impl PortalTrait for Client {
    fn _send_encode_portal_create<I>(&self, name: &str, stmt: &Statement, params: I) -> Result<Response, Error>
    where
        I: AsParams,
    {
        codec::send_encode_portal_create(&self.tx, name, stmt, params.into_iter())
    }

    fn _send_encode_portal_query(&self, name: &str, max_rows: i32) -> Result<Response, Error> {
        codec::send_encode_portal_query(&self.tx, name, max_rows)
    }

    fn _send_encode_portal_cancel(&self, name: &str) {
        let _ = codec::send_encode_portal_cancel(&self.tx, name);
    }
}

impl Query for Client {
    #[inline]
    fn _send_encode_query<I>(&self, stmt: &Statement, params: I) -> Result<Response, Error>
    where
        I: AsParams,
    {
        codec::send_encode_query(&self.tx, stmt, params)
    }
}

impl QuerySimple for Client {
    #[inline]
    fn _send_encode_query_simple(&self, stmt: &str) -> Result<Response, Error> {
        codec::send_encode_query_simple(&self.tx, stmt)
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
