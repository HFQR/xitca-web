use std::collections::HashMap;

use postgres_types::{Oid, Type};
use xitca_io::bytes::BytesMut;
use xitca_unsafe_collection::no_hash::NoHashBuilder;

use tokio::sync::mpsc::UnboundedSender;

use super::{error::Error, request::Request, response::Response, statement::Statement, util::lock::Lock};

pub struct Client {
    pub(crate) tx: UnboundedSender<Request>,
    pub(crate) buf: Lock<BytesMut>,
    cached_typeinfo: Lock<CachedTypeInfo>,
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
    pub(crate) fn new(tx: UnboundedSender<Request>) -> Self {
        Self {
            tx,
            buf: Lock::new(BytesMut::with_capacity(4096)),
            cached_typeinfo: Lock::new(CachedTypeInfo {
                typeinfo: None,
                typeinfo_composite: None,
                typeinfo_enum: None,
                types: HashMap::default(),
            }),
        }
    }

    pub fn closed(&self) -> bool {
        self.tx.is_closed()
    }

    pub(crate) fn send(&self, msg: BytesMut) -> Result<Response, Error> {
        let (req, res) = Request::new_pair(msg);
        self.tx.send(req)?;
        Ok(res)
    }

    // send a message to database without concerning a response.
    pub(crate) fn send2(&self, msg: BytesMut) -> Result<(), Error> {
        let req = Request::new(msg);
        self.tx.send(req)?;
        Ok(())
    }

    pub fn typeinfo(&self) -> Option<Statement> {
        self.cached_typeinfo.lock().typeinfo.clone()
    }

    pub fn set_typeinfo(&self, statement: &Statement) {
        self.cached_typeinfo.lock().typeinfo = Some(statement.clone());
    }

    pub fn typeinfo_composite(&self) -> Option<Statement> {
        self.cached_typeinfo.lock().typeinfo_composite.clone()
    }

    pub fn set_typeinfo_composite(&self, statement: &Statement) {
        self.cached_typeinfo.lock().typeinfo_composite = Some(statement.clone());
    }

    pub fn typeinfo_enum(&self) -> Option<Statement> {
        self.cached_typeinfo.lock().typeinfo_enum.clone()
    }

    pub fn set_typeinfo_enum(&self, statement: &Statement) {
        self.cached_typeinfo.lock().typeinfo_enum = Some(statement.clone());
    }

    pub fn type_(&self, oid: Oid) -> Option<Type> {
        self.cached_typeinfo.lock().types.get(&oid).cloned()
    }

    pub fn set_type(&self, oid: Oid, type_: &Type) {
        self.cached_typeinfo.lock().types.insert(oid, type_.clone());
    }

    pub fn clear_type_cache(&self) {
        self.cached_typeinfo.lock().types.clear();
    }

    pub(crate) fn with_buf_fallible<F, T, E>(&self, f: F) -> Result<T, E>
    where
        F: FnOnce(&mut BytesMut) -> Result<T, E>,
    {
        let mut buf = self.buf.lock();
        let res = f(&mut buf);
        if res.is_err() {
            buf.clear();
        }
        res
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // convert leaked statements to guarded statements.
        // this is to cancel the statement on client go away.
        let (type_info, typeinfo_composite, typeinfo_enum) = {
            let cache = self.cached_typeinfo.get_mut();
            (
                cache.typeinfo.take(),
                cache.typeinfo_composite.take(),
                cache.typeinfo_enum.take(),
            )
        };

        if let Some(stmt) = type_info {
            drop(stmt.into_guarded(self));
        }

        if let Some(stmt) = typeinfo_composite {
            drop(stmt.into_guarded(self));
        }

        if let Some(stmt) = typeinfo_enum {
            drop(stmt.into_guarded(self));
        }
    }
}
