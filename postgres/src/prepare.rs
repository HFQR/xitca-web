use std::sync::atomic::{AtomicUsize, Ordering};

use fallible_iterator::FallibleIterator;
use postgres_protocol::message::{backend, frontend};
use postgres_types::{Field, Kind, Oid, Type};
use tracing::debug;
use xitca_io::bytes::{Bytes, BytesMut};

use super::{
    client::Client,
    error::Error,
    statement::{Column, Statement, StatementGuarded},
};

impl Client {
    pub async fn prepare(&self, query: &str, types: &[Type]) -> Result<StatementGuarded<'_>, Error> {
        let buf = prepare_buf(&mut *self.buf.borrow_mut(), query, types)?;

        let mut res = self.send(buf).await?;

        match res.recv().await? {
            backend::Message::ParseComplete => {}
            _ => return Err(Error::ToDo),
        }

        let parameter_description = match res.recv().await? {
            backend::Message::ParameterDescription(body) => body,
            _ => return Err(Error::ToDo),
        };

        let row_description = match res.recv().await? {
            backend::Message::RowDescription(body) => Some(body),
            backend::Message::NoData => None,
            _ => return Err(Error::ToDo),
        };

        let mut parameters = vec![];
        let mut it = parameter_description.parameters();
        while let Some(oid) = it.next().map_err(|_| Error::ToDo)? {
            let ty = get_type(self, oid).await?;
            parameters.push(ty);
        }

        let mut columns = vec![];
        if let Some(row_description) = row_description {
            let mut it = row_description.fields();
            while let Some(field) = it.next().map_err(|_| Error::ToDo)? {
                let type_ = get_type(self, field.type_oid()).await?;
                let column = Column::new(field.name().to_string(), type_);
                columns.push(column);
            }
        }

        todo!()

        // Ok(Statement::new(client, name, parameters, columns))
    }

    async fn _prepare(&self, query: &str, types: &[Type]) -> Result<Statement, Error> {
        let buf = prepare_buf(&mut *self.buf.borrow_mut(), query, types)?;

        let mut res = self.send(buf).await?;

        match res.recv().await? {
            backend::Message::ParseComplete => {}
            _ => return Err(Error::ToDo),
        }

        let parameter_description = match res.recv().await? {
            backend::Message::ParameterDescription(body) => body,
            _ => return Err(Error::ToDo),
        };

        let row_description = match res.recv().await? {
            backend::Message::RowDescription(body) => Some(body),
            backend::Message::NoData => None,
            _ => return Err(Error::ToDo),
        };

        let mut parameters = vec![];
        let mut it = parameter_description.parameters();
        while let Some(oid) = it.next().map_err(|_| Error::ToDo)? {
            let ty = get_type(self, oid).await?;
            parameters.push(ty);
        }

        let mut columns = vec![];
        if let Some(row_description) = row_description {
            let mut it = row_description.fields();
            while let Some(field) = it.next().map_err(|_| Error::ToDo)? {
                let type_ = get_type(self, field.type_oid()).await?;
                let column = Column::new(field.name().to_string(), type_);
                columns.push(column);
            }
        }

        todo!()

        // Ok(Statement::new(client, name, parameters, columns))
    }
}

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

fn prepare_buf(buf: &mut BytesMut, query: &str, types: &[Type]) -> Result<Bytes, Error> {
    let name = &format!("s{}", NEXT_ID.fetch_add(1, Ordering::SeqCst));

    if types.is_empty() {
        debug!("preparing query {}: {}", name, query);
    } else {
        debug!("preparing query {} with types {:?}: {}", name, types, query);
    }

    frontend::parse(name, query, types.iter().map(Type::oid), buf)?;
    frontend::describe(b'S', name, buf)?;
    frontend::sync(buf);

    Ok(buf.split().freeze())
}

async fn get_type(client: &Client, oid: Oid) -> Result<Type, Error> {
    if let Some(type_) = Type::from_oid(oid) {
        return Ok(type_);
    }

    todo!()

    // let stmt = typeinfo_statement(client).await?;

    // let rows = query::query(client, stmt, slice_iter(&[&oid])).await?;
    // pin!(rows);

    // let row = match rows.try_next().await? {
    //     Some(row) => row,
    //     None => return Err(Error::unexpected_message()),
    // };

    // let name: String = row.try_get(0)?;
    // let type_: i8 = row.try_get(1)?;
    // let elem_oid: Oid = row.try_get(2)?;
    // let rngsubtype: Option<Oid> = row.try_get(3)?;
    // let basetype: Oid = row.try_get(4)?;
    // let schema: String = row.try_get(5)?;
    // let relid: Oid = row.try_get(6)?;

    // let kind = if type_ == b'e' as i8 {
    //     let variants = get_enum_variants(client, oid).await?;
    //     Kind::Enum(variants)
    // } else if type_ == b'p' as i8 {
    //     Kind::Pseudo
    // } else if basetype != 0 {
    //     let type_ = get_type_rec(client, basetype).await?;
    //     Kind::Domain(type_)
    // } else if elem_oid != 0 {
    //     let type_ = get_type_rec(client, elem_oid).await?;
    //     Kind::Array(type_)
    // } else if relid != 0 {
    //     let fields = get_composite_fields(client, relid).await?;
    //     Kind::Composite(fields)
    // } else if let Some(rngsubtype) = rngsubtype {
    //     let type_ = get_type_rec(client, rngsubtype).await?;
    //     Kind::Range(type_)
    // } else {
    //     Kind::Simple
    // };

    // let type_ = Type::new(name, oid, kind, schema);
    // client.set_type(oid, &type_);

    // Ok(type_)
}

impl Client {
    async fn typeinfo_statement(&self) -> Result<Statement, Error> {
        if let Some(stmt) = self.typeinfo() {
            return Ok(stmt);
        }

        let stmt = match Box::pin(self._prepare(TYPEINFO_QUERY, &[])).await {
            Ok(stmt) => stmt,
            // TODO: add fallback handling.
            // Err(ref e) if e.code() == Some(&SqlState::UNDEFINED_TABLE) => {
            //     Box::pin(self.prepare(TYPEINFO_FALLBACK_QUERY, &[])).await?
            // }
            Err(e) => return Err(e),
        };

        self.set_typeinfo(stmt);

        Ok(self.typeinfo().unwrap())
    }
}

const TYPEINFO_QUERY: &str = "\
SELECT t.typname, t.typtype, t.typelem, r.rngsubtype, t.typbasetype, n.nspname, t.typrelid
FROM pg_catalog.pg_type t
LEFT OUTER JOIN pg_catalog.pg_range r ON r.rngtypid = t.oid
INNER JOIN pg_catalog.pg_namespace n ON t.typnamespace = n.oid
WHERE t.oid = $1
";

// Range types weren't added until Postgres 9.2, so pg_range may not exist
const TYPEINFO_FALLBACK_QUERY: &str = "\
SELECT t.typname, t.typtype, t.typelem, NULL::OID, t.typbasetype, n.nspname, t.typrelid
FROM pg_catalog.pg_type t
INNER JOIN pg_catalog.pg_namespace n ON t.typnamespace = n.oid
WHERE t.oid = $1
";

const TYPEINFO_ENUM_QUERY: &str = "\
SELECT enumlabel
FROM pg_catalog.pg_enum
WHERE enumtypid = $1
ORDER BY enumsortorder
";

// Postgres 9.0 didn't have enumsortorder
const TYPEINFO_ENUM_FALLBACK_QUERY: &str = "\
SELECT enumlabel
FROM pg_catalog.pg_enum
WHERE enumtypid = $1
ORDER BY oid
";

const TYPEINFO_COMPOSITE_QUERY: &str = "\
SELECT attname, atttypid
FROM pg_catalog.pg_attribute
WHERE attrelid = $1
AND NOT attisdropped
AND attnum > 0
ORDER BY attnum
";
