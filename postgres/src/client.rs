use core::future::Future;

use std::{collections::HashMap, sync::Mutex};

use postgres_protocol::message::frontend;
use postgres_types::{Oid, Type};
use xitca_unsafe_collection::no_hash::NoHashBuilder;

use super::{
    driver::{codec::Response, DriverTx},
    error::Error,
    iter::slice_iter,
    query::{encode, AsParams, Query, QuerySimple, RowSimpleStream, RowStream},
    session::Session,
    statement::Statement,
    transaction::Transaction,
    ToSql,
};

pub struct Client {
    pub(crate) tx: DriverTx,
    pub(crate) session: Session,
    cached_typeinfo: Mutex<CachedTypeInfo>,
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
    pub(crate) fn new(tx: DriverTx, session: Session) -> Self {
        Self {
            tx,
            session,
            cached_typeinfo: Mutex::new(CachedTypeInfo {
                typeinfo: None,
                typeinfo_composite: None,
                typeinfo_enum: None,
                types: HashMap::default(),
            }),
        }
    }

    /// Executes a statement, returning a vector of the resulting rows.
    ///
    /// A statement may contain parameters, specified by `$n`, where `n` is the index of the
    /// parameter of the list provided, 1-indexed.
    ///
    /// If the same statement will be repeatedly executed (perhaps with different query parameters),
    /// consider preparing the statement up front with the [Client::prepare] method.
    ///
    /// # Panics
    ///
    /// Panics if given params slice length does not match the length of [Statement::params].
    #[inline]
    pub fn query<'a>(&self, stmt: &'a Statement, params: &[&(dyn ToSql + Sync)]) -> Result<RowStream<'a>, Error> {
        (&mut &*self)._query(stmt, params)
    }

    /// # Panics
    ///
    /// Panics if given params' [ExactSizeIterator::len] does not match the length of [Statement::params].
    pub fn query_raw<'a, I>(&self, stmt: &'a Statement, params: I) -> Result<RowStream<'a>, Error>
    where
        I: AsParams,
    {
        (&mut &*self)._query_raw(stmt, params)
    }

    /// Executes a statement, returning the number of rows modified.
    ///
    /// A statement may contain parameters, specified by `$n`, where `n` is the index of the parameter of the list
    /// provided, 1-indexed.
    ///
    /// If the same statement will be repeatedly executed (perhaps with different query parameters),
    /// consider preparing the statement up front with the [Client::prepare] method.
    ///
    /// If the statement does not modify any rows (e.g. `SELECT`), 0 is returned.
    ///
    /// # Panics
    ///
    /// Panics if given params' [ExactSizeIterator::len] does not match the length of [Statement::params].
    pub fn execute(
        &self,
        stmt: &Statement,
        params: &[&(dyn ToSql + Sync)],
    ) -> impl Future<Output = Result<u64, Error>> + Send {
        // TODO: call execute_raw when Rust2024 edition capture rule is stabled.
        let res = (&mut &*self)._send_encode(stmt, slice_iter(params));
        async { res?.try_into_row_affected().await }
    }

    /// # Panics
    ///
    /// Panics if given params' [ExactSizeIterator::len] does not match the length of [Statement::params].
    #[inline]
    pub fn execute_raw<I>(&self, stmt: &Statement, params: I) -> impl Future<Output = Result<u64, Error>> + Send
    where
        I: AsParams,
    {
        let res = (&mut &*self)._send_encode(stmt, params);
        async { res?.try_into_row_affected().await }
    }

    #[inline]
    pub fn query_simple(&self, stmt: &str) -> Result<RowSimpleStream, Error> {
        (&mut &*self)._query_simple(stmt)
    }

    #[inline]
    pub fn execute_simple(&self, stmt: &str) -> impl Future<Output = Result<u64, Error>> {
        (&mut &*self)._execute_simple(stmt)
    }

    pub fn transaction(&mut self) -> impl Future<Output = Result<Transaction<Client>, Error>> {
        Transaction::new(self)
    }

    /// a lossy hint of running state of io driver. an io driver shutdown can happen
    /// at the same time this api is called.
    pub fn closed(&self) -> bool {
        self.tx.is_closed()
    }

    pub fn typeinfo(&self) -> Option<Statement> {
        self.cached_typeinfo.lock().unwrap().typeinfo.clone()
    }

    pub fn set_typeinfo(&self, statement: &Statement) {
        self.cached_typeinfo.lock().unwrap().typeinfo = Some(statement.clone());
    }

    pub fn typeinfo_composite(&self) -> Option<Statement> {
        self.cached_typeinfo.lock().unwrap().typeinfo_composite.clone()
    }

    pub fn set_typeinfo_composite(&self, statement: &Statement) {
        self.cached_typeinfo.lock().unwrap().typeinfo_composite = Some(statement.clone());
    }

    pub fn typeinfo_enum(&self) -> Option<Statement> {
        self.cached_typeinfo.lock().unwrap().typeinfo_enum.clone()
    }

    pub fn set_typeinfo_enum(&self, statement: &Statement) {
        self.cached_typeinfo.lock().unwrap().typeinfo_enum = Some(statement.clone());
    }

    pub fn type_(&self, oid: Oid) -> Option<Type> {
        self.cached_typeinfo.lock().unwrap().types.get(&oid).cloned()
    }

    pub fn set_type(&self, oid: Oid, type_: &Type) {
        self.cached_typeinfo.lock().unwrap().types.insert(oid, type_.clone());
    }

    pub fn clear_type_cache(&self) {
        self.cached_typeinfo.lock().unwrap().types.clear();
    }
}

impl Query for &Client {
    #[inline]
    fn _send_encode<I>(&mut self, stmt: &Statement, params: I) -> Result<Response, Error>
    where
        I: AsParams,
    {
        send_encode(self, stmt, params)
    }
}

impl Query for Client {
    #[inline]
    fn _send_encode<I>(&mut self, stmt: &Statement, params: I) -> Result<Response, Error>
    where
        I: AsParams,
    {
        send_encode(self, stmt, params)
    }
}

fn send_encode<I>(cli: &Client, stmt: &Statement, params: I) -> Result<Response, Error>
where
    I: AsParams,
{
    cli.tx.send(|buf| encode::encode(buf, stmt, params.into_iter()))
}

impl QuerySimple for Client {
    #[inline]
    fn _send_encode_simple(&mut self, stmt: &str) -> Result<Response, Error> {
        send_encode_simple(self, stmt)
    }
}

impl QuerySimple for &Client {
    #[inline]
    fn _send_encode_simple(&mut self, stmt: &str) -> Result<Response, Error> {
        send_encode_simple(self, stmt)
    }
}

fn send_encode_simple(cli: &Client, stmt: &str) -> Result<Response, Error> {
    cli.tx.send(|buf| frontend::query(stmt, buf).map_err(Into::into))
}

impl Drop for Client {
    fn drop(&mut self) {
        // convert leaked statements to guarded statements.
        // this is to cancel the statement on client go away.
        let (type_info, typeinfo_composite, typeinfo_enum) = {
            let cache = self.cached_typeinfo.get_mut().unwrap();
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
