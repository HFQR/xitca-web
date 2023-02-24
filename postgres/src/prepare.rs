use core::{future::Future, pin::Pin};

use std::sync::atomic::{AtomicUsize, Ordering};

use fallible_iterator::FallibleIterator;
use postgres_protocol::message::{backend, frontend};
use postgres_types::{Field, Kind, Oid};
use tracing::debug;
use xitca_io::bytes::BytesMut;

use crate::{
    client::Client,
    column::Column,
    error::Error,
    iter::AsyncIterator,
    statement::{Statement, StatementGuarded},
    Type,
};

#[cfg(feature = "single-thread")]
type BoxedFuture<'a> = Pin<Box<dyn Future<Output = Result<Type, Error>> + 'a>>;

#[cfg(not(feature = "single-thread"))]
type BoxedFuture<'a> = Pin<Box<dyn Future<Output = Result<Type, Error>> + Send + 'a>>;

impl Client {
    pub async fn prepare(&self, query: &str, types: &[Type]) -> Result<StatementGuarded<'_>, Error> {
        self._prepare(query, types).await.map(|stmt| stmt.into_guarded(self))
    }
}

impl Client {
    async fn _prepare(&self, query: &str, types: &[Type]) -> Result<Statement, Error> {
        let name = format!("s{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));

        let buf = self.prepare_buf(name.as_str(), query, types)?;

        let mut res = self.send(buf)?;

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

        let mut parameters = Vec::new();
        let mut it = parameter_description.parameters();
        while let Some(oid) = it.next().map_err(|_| Error::ToDo)? {
            let ty = self.get_type(oid).await?;
            parameters.push(ty);
        }

        let mut columns = Vec::new();
        if let Some(row_description) = row_description {
            let mut it = row_description.fields();
            while let Some(field) = it.next().map_err(|_| Error::ToDo)? {
                let type_ = self.get_type(field.type_oid()).await?;
                let column = Column::new(field.name(), type_);
                columns.push(column);
            }
        }

        Ok(Statement::new(name, parameters, columns))
    }

    // get type is called recursively so a boxed future is needed.
    #[inline(never)]
    fn get_type(&self, oid: Oid) -> BoxedFuture<'_> {
        Box::pin(async move {
            if let Some(ty) = Type::from_oid(oid).or_else(|| self.type_(oid)) {
                return Ok(ty);
            }

            let stmt = self.typeinfo_statement().await?;

            let mut rows = self.query_raw(&stmt, &[&oid]).await?;
            let row = rows.next().await.ok_or(Error::ToDo)??;

            let name = row.try_get::<String>(0)?;
            let type_ = row.try_get::<i8>(1)?;
            let elem_oid = row.try_get::<Oid>(2)?;
            let rngsubtype = row.try_get::<Option<Oid>>(3)?;
            let basetype = row.try_get::<Oid>(4)?;
            let schema = row.try_get::<String>(5)?;
            let relid = row.try_get::<Oid>(6)?;

            let kind = if type_ == b'e' as i8 {
                let variants = self.get_enum_variants(oid).await?;
                Kind::Enum(variants)
            } else if type_ == b'p' as i8 {
                Kind::Pseudo
            } else if basetype != 0 {
                let type_ = self.get_type(basetype).await?;
                Kind::Domain(type_)
            } else if elem_oid != 0 {
                let type_ = self.get_type(elem_oid).await?;
                Kind::Array(type_)
            } else if relid != 0 {
                let fields = self.get_composite_fields(relid).await?;
                Kind::Composite(fields)
            } else if let Some(rngsubtype) = rngsubtype {
                let type_ = self.get_type(rngsubtype).await?;
                Kind::Range(type_)
            } else {
                Kind::Simple
            };

            let type_ = Type::new(name, oid, kind, schema);
            self.set_type(oid, &type_);

            Ok(type_)
        })
    }

    #[inline(never)]
    async fn typeinfo_statement(&self) -> Result<Statement, Error> {
        if let Some(stmt) = self.typeinfo() {
            return Ok(stmt);
        }

        let stmt = match self._prepare(TYPEINFO_QUERY, &[]).await {
            Ok(stmt) => stmt,
            Err(_) => {
                self._prepare(TYPEINFO_FALLBACK_QUERY, &[]).await?
            }
            // Err(ref e) if e.code() == Some(&SqlState::UNDEFINED_TABLE) => {
            //     self._prepare_boxed(TYPEINFO_FALLBACK_QUERY, &[]).await?
            // }
            // Err(e) => return Err(e),
        };

        self.set_typeinfo(&stmt);

        Ok(stmt)
    }

    #[inline(never)]
    async fn get_enum_variants(&self, oid: Oid) -> Result<Vec<String>, Error> {
        let stmt = self.typeinfo_enum_statement().await?;

        let mut rows = self.query_raw(&stmt, &[&oid]).await?;

        let mut res = Vec::new();

        while let Some(row) = rows.next().await {
            let variant = row?.try_get(0)?;
            res.push(variant);
        }

        Ok(res)
    }

    #[inline(never)]
    async fn typeinfo_enum_statement(&self) -> Result<Statement, Error> {
        if let Some(stmt) = self.typeinfo_enum() {
            return Ok(stmt);
        }

        let stmt = match self._prepare(TYPEINFO_ENUM_QUERY, &[]).await {
            Ok(stmt) => stmt,
            Err(_) => self._prepare(TYPEINFO_ENUM_FALLBACK_QUERY, &[]).await?,
            // Err(ref e) if e.code() == Some(&SqlState::UNDEFINED_COLUMN) => {
            //     prepare_rec(client, TYPEINFO_ENUM_FALLBACK_QUERY, &[]).await?
            // }
            // Err(e) => return Err(e),
        };

        self.set_typeinfo_enum(&stmt);

        Ok(stmt)
    }

    #[inline(never)]
    async fn get_composite_fields(&self, oid: Oid) -> Result<Vec<Field>, Error> {
        let stmt = self.typeinfo_composite_statement().await?;

        let mut rows = self.query_raw(&stmt, &[&oid]).await?;

        let mut fields = Vec::new();

        while let Some(row) = rows.next().await {
            let row = row?;
            let name = row.try_get(0)?;
            let oid = row.try_get(1)?;
            let type_ = self.get_type(oid).await?;
            fields.push(Field::new(name, type_));
        }

        Ok(fields)
    }

    #[inline(never)]
    async fn typeinfo_composite_statement(&self) -> Result<Statement, Error> {
        if let Some(stmt) = self.typeinfo_composite() {
            return Ok(stmt);
        }

        let stmt = self._prepare(TYPEINFO_COMPOSITE_QUERY, &[]).await?;

        self.set_typeinfo_composite(&stmt);

        Ok(stmt)
    }

    fn prepare_buf(&self, name: &str, query: &str, types: &[Type]) -> Result<BytesMut, Error> {
        if types.is_empty() {
            debug!("preparing query {}: {}", name, query);
        } else {
            debug!("preparing query {} with types {:?}: {}", name, types, query);
        }

        self.try_encode_with(|buf| {
            frontend::parse(name, query, types.iter().map(Type::oid), buf)?;
            frontend::describe(b'S', name, buf)?;
            frontend::sync(buf);
            Ok(())
        })
    }
}

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

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
