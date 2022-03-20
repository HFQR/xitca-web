use std::{cell::RefCell, collections::HashMap};

use postgres_types::{Oid, Type};
use tokio::sync::mpsc::{unbounded_channel, Sender};
use xitca_io::bytes::{Bytes, BytesMut};

use super::{error::Error, request::Request, response::Response, statement::Statement};

pub struct Client {
    pub(crate) tx: Sender<Request>,
    pub(crate) buf: RefCell<BytesMut>,
    cached_typeinfo: RefCell<CachedTypeInfo>,
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
    types: HashMap<Oid, Type>,
}

impl Client {
    pub(crate) fn new(tx: Sender<Request>) -> Self {
        Self {
            tx,
            buf: RefCell::new(BytesMut::new()),
            cached_typeinfo: RefCell::new(CachedTypeInfo {
                typeinfo: None,
                typeinfo_composite: None,
                typeinfo_enum: None,
                types: HashMap::new(),
            }),
        }
    }

    pub async fn query(&self) -> Result<(), Error> {
        todo!()
    }

    pub fn closed(&self) -> bool {
        self.tx.is_closed()
    }

    pub(crate) async fn send(&self, msg: Bytes) -> Result<Response, Error> {
        let (tx, rx) = unbounded_channel();

        self.tx
            .send(Request { tx, msg })
            .await
            .map_err(|_| Error::ConnectionClosed)?;

        Ok(Response::new(rx))
    }

    pub fn typeinfo(&self) -> Option<Statement> {
        self.cached_typeinfo.borrow().typeinfo.as_ref().cloned()
    }

    pub fn set_typeinfo(&self, statement: Statement) {
        self.cached_typeinfo.borrow_mut().typeinfo = Some(statement);
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // convert leaked statements to guarded statements.
        // this is to cancel the statement on client go away.

        if let Some(stmt) = self.cached_typeinfo.get_mut().typeinfo.take() {
            drop(stmt.into_guarded(&self));
        }

        if let Some(stmt) = self.cached_typeinfo.get_mut().typeinfo_composite.take() {
            drop(stmt.into_guarded(&self));
        }

        if let Some(stmt) = self.cached_typeinfo.get_mut().typeinfo_enum.take() {
            drop(stmt.into_guarded(&self));
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn assert_send<C: Send>() {}

    #[test]
    fn is_send() {
        assert_send::<Client>();
    }
}
