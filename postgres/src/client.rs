use core::future::Future;

use std::{collections::HashMap, sync::Mutex};

use xitca_unsafe_collection::no_hash::NoHashBuilder;

use super::{
    copy::{CopyIn, CopyOut},
    driver::DriverTx,
    error::Error,
    session::Session,
    statement::Statement,
    transaction::{Transaction, TransactionBuilder},
    types::{Oid, Type},
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
/// use xitca_postgres::{dev::ClientBorrowMut, transaction::TransactionBuilder, Client, Error};
/// # use xitca_postgres::dev::{Encode, Prepare, Query, Response};
/// # use xitca_postgres::types::{Oid, Type};
///
/// // a client wrapper use reference counted smart pointer.
/// // it's easy to create multiple instance of &mut SharedClient with help of cloning of smart pointer
/// // and none of them can be used correctly with Transaction nor CopyIn
/// #[derive(Clone)]
/// struct SharedClient(Arc<Client>);
///
/// # impl Query for SharedClient {
/// #     fn _send_encode_query<S>(&self, stmt: S) -> Result<(S::Output, Response), Error>
/// #     where
/// #         S: Encode,
/// #     {
/// #         self.0._send_encode_query(stmt)
/// #     }
/// # }
/// # impl Prepare for SharedClient {
/// #     async fn _get_type(&self, oid: Oid) -> Result<Type, Error> {
/// #         self.0._get_type(oid).await
/// #     }
/// #     fn _get_type_blocking(&self, oid: Oid) -> Result<Type, Error> {
/// #         self.0._get_type_blocking(oid)
/// #     }
/// # }
///
/// // client new type has to impl this trait to mark they can truly offer a mutable reference to Client
/// //
/// impl ClientBorrowMut for SharedClient {
///     fn _borrow_mut(&mut self) -> &mut Client {
///         Arc::get_mut(&mut self.0).expect("you can't safely implement this trait with SharedClient.")
///     }
/// }
///
/// async fn borrow_mut(cli: Client) -> Result<(), Error> {
///     let mut cli = SharedClient(Arc::new(cli));
///
///     // this line works fine as shared client is exclusivly borrowed by transaction
///     let _ = TransactionBuilder::new().begin(&mut cli).await?;    
///
///     // clone shared client before starting a transaction.
///     let _cli2 = cli.clone();
///
///     // this line will panic as shared client has another copy being held by cli2 varaiable
///     let _ = TransactionBuilder::new().begin(&mut cli).await?;
///
///     Ok(())
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
    /// start a transaction
    #[inline]
    pub fn transaction(&mut self) -> impl Future<Output = Result<Transaction<'_, Self>, Error>> + Send {
        TransactionBuilder::new().begin(self)
    }

    /// Executes a `COPY FROM STDIN` statement, returning a sink used to write the copy data.
    ///
    /// PostgreSQL does not support parameters in `COPY` statements, so this method does not take any. The copy *must*
    /// be explicitly completed via [`CopyIn::finish`]. If it is not, the copy will be aborted.
    #[inline]
    pub fn copy_in(&mut self, stmt: &Statement) -> impl Future<Output = Result<CopyIn<'_, Self>, Error>> + Send {
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

    /// a lossy hint of running state of io driver. an io driver shutdown can happen at the same time this api is called.
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
    #[inline]
    fn _borrow_mut(&mut self) -> &mut Client {
        self
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
