use core::future::Future;

use postgres_types::{Field, Kind, Oid};

use super::{
    client::Client,
    error::{DbError, Error, SqlState},
    execute::{Execute, ExecuteBlocking},
    iter::AsyncLendingIterator,
    query::Query,
    statement::{Statement, StatementNamed},
    types::Type,
    BoxedFuture,
};

/// trait generic over preparing statement and canceling of prepared statement
pub trait Prepare: Query + Sync {
    // get type is called recursively so a boxed future is needed.
    fn _get_type(&self, oid: Oid) -> impl Future<Output = Result<Type, Error>> + Send;

    #[doc(hidden)]
    fn _get_type_boxed(&self, oid: Oid) -> BoxedFuture<'_, Result<Type, Error>> {
        Box::pin(self._get_type(oid))
    }

    // blocking version of [`Prepare::_get_type`].
    fn _get_type_blocking(&self, oid: Oid) -> Result<Type, Error>;
}

impl Prepare for Client {
    async fn _get_type(&self, oid: Oid) -> Result<Type, Error> {
        if let Some(ty) = Type::from_oid(oid).or_else(|| self.type_(oid)) {
            return Ok(ty);
        }

        let stmt = self.typeinfo_statement().await?;

        let mut rows = stmt.bind([oid]).query(self).await?;
        let row = rows.try_next().await?.ok_or_else(Error::unexpected)?;

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
            let type_ = self._get_type_boxed(basetype).await?;
            Kind::Domain(type_)
        } else if elem_oid != 0 {
            let type_ = self._get_type_boxed(elem_oid).await?;
            Kind::Array(type_)
        } else if relid != 0 {
            let fields = self.get_composite_fields(relid).await?;
            Kind::Composite(fields)
        } else if let Some(rngsubtype) = rngsubtype {
            let type_ = self._get_type_boxed(rngsubtype).await?;
            Kind::Range(type_)
        } else {
            Kind::Simple
        };

        let type_ = Type::new(name, oid, kind, schema);
        self.set_type(oid, &type_);

        Ok(type_)
    }

    fn _get_type_blocking(&self, oid: Oid) -> Result<Type, Error> {
        if let Some(ty) = Type::from_oid(oid).or_else(|| self.type_(oid)) {
            return Ok(ty);
        }

        let stmt = self.typeinfo_statement_blocking()?;

        let rows = stmt.bind([oid]).query_blocking(self)?;
        let row = rows.into_iter().next().ok_or_else(Error::unexpected)??;

        let name = row.try_get::<String>(0)?;
        let type_ = row.try_get::<i8>(1)?;
        let elem_oid = row.try_get::<Oid>(2)?;
        let rngsubtype = row.try_get::<Option<Oid>>(3)?;
        let basetype = row.try_get::<Oid>(4)?;
        let schema = row.try_get::<String>(5)?;
        let relid = row.try_get::<Oid>(6)?;

        let kind = if type_ == b'e' as i8 {
            let variants = self.get_enum_variants_blocking(oid)?;
            Kind::Enum(variants)
        } else if type_ == b'p' as i8 {
            Kind::Pseudo
        } else if basetype != 0 {
            let type_ = self._get_type_blocking(basetype)?;
            Kind::Domain(type_)
        } else if elem_oid != 0 {
            let type_ = self._get_type_blocking(elem_oid)?;
            Kind::Array(type_)
        } else if relid != 0 {
            let fields = self.get_composite_fields_blocking(relid)?;
            Kind::Composite(fields)
        } else if let Some(rngsubtype) = rngsubtype {
            let type_ = self._get_type_blocking(rngsubtype)?;
            Kind::Range(type_)
        } else {
            Kind::Simple
        };

        let type_ = Type::new(name, oid, kind, schema);
        self.set_type(oid, &type_);

        Ok(type_)
    }
}

const TYPEINFO_QUERY: StatementNamed = Statement::named(
    "SELECT t.typname, t.typtype, t.typelem, r.rngsubtype, t.typbasetype, n.nspname, t.typrelid \
    FROM pg_catalog.pg_type t \
    LEFT OUTER JOIN pg_catalog.pg_range r ON r.rngtypid = t.oid \
    INNER JOIN pg_catalog.pg_namespace n ON t.typnamespace = n.oid \
    WHERE t.oid = $1",
    &[],
);

// Range types weren't added until Postgres 9.2, so pg_range may not exist
const TYPEINFO_FALLBACK_QUERY: StatementNamed = Statement::named(
    "SELECT t.typname, t.typtype, t.typelem, NULL::OID, t.typbasetype, n.nspname, t.typrelid \
    FROM pg_catalog.pg_type t \
    INNER JOIN pg_catalog.pg_namespace n ON t.typnamespace = n.oid \
    WHERE t.oid = $1",
    &[],
);

const TYPEINFO_ENUM_QUERY: StatementNamed = Statement::named(
    "SELECT enumlabel \
    FROM pg_catalog.pg_enum \
    WHERE enumtypid = $1 \
    ORDER BY enumsortorder",
    &[],
);

// Postgres 9.0 didn't have enumsortorder
const TYPEINFO_ENUM_FALLBACK_QUERY: StatementNamed = Statement::named(
    "SELECT enumlabel \
    FROM pg_catalog.pg_enum \
    WHERE enumtypid = $1 \
    ORDER BY oid",
    &[],
);

const TYPEINFO_COMPOSITE_QUERY: StatementNamed = Statement::named(
    "SELECT attname, atttypid \
    FROM pg_catalog.pg_attribute \
    WHERE attrelid = $1 \
    AND NOT attisdropped \
    AND attnum > 0 \
    ORDER BY attnum",
    &[],
);

impl Client {
    async fn get_enum_variants(&self, oid: Oid) -> Result<Vec<String>, Error> {
        let stmt = self.typeinfo_enum_statement().await?;
        let mut rows = stmt.bind([oid]).query(self).await?;
        let mut res = Vec::new();
        while let Some(row) = rows.try_next().await? {
            let variant = row.try_get(0)?;
            res.push(variant);
        }
        Ok(res)
    }

    async fn get_composite_fields(&self, oid: Oid) -> Result<Vec<Field>, Error> {
        let stmt = self.typeinfo_composite_statement().await?;
        let mut rows = stmt.bind([oid]).query(self).await?;
        let mut fields = Vec::new();
        while let Some(row) = rows.try_next().await? {
            let name = row.try_get(0)?;
            let oid = row.try_get(1)?;
            let type_ = self._get_type_boxed(oid).await?;
            fields.push(Field::new(name, type_));
        }
        Ok(fields)
    }

    async fn typeinfo_statement(&self) -> Result<Statement, Error> {
        if let Some(stmt) = self.typeinfo() {
            return Ok(stmt);
        }
        let stmt = match TYPEINFO_QUERY.execute(self).await.map(|stmt| stmt.leak()) {
            Ok(stmt) => stmt,
            Err(e) => {
                return if e
                    .downcast_ref::<DbError>()
                    .is_some_and(|e| SqlState::UNDEFINED_TABLE.eq(e.code()))
                {
                    TYPEINFO_FALLBACK_QUERY.execute(self).await.map(|stmt| stmt.leak())
                } else {
                    Err(e)
                }
            }
        };
        self.set_typeinfo(&stmt);
        Ok(stmt)
    }

    async fn typeinfo_enum_statement(&self) -> Result<Statement, Error> {
        if let Some(stmt) = self.typeinfo_enum() {
            return Ok(stmt);
        }
        let stmt = match TYPEINFO_ENUM_QUERY.execute(self).await {
            Ok(stmt) => stmt.leak(),
            Err(e) => {
                return if e
                    .downcast_ref::<DbError>()
                    .is_some_and(|e| SqlState::UNDEFINED_COLUMN.eq(e.code()))
                {
                    TYPEINFO_ENUM_FALLBACK_QUERY.execute(self).await.map(|stmt| stmt.leak())
                } else {
                    Err(e)
                }
            }
        };
        self.set_typeinfo_enum(&stmt);
        Ok(stmt)
    }

    async fn typeinfo_composite_statement(&self) -> Result<Statement, Error> {
        if let Some(stmt) = self.typeinfo_composite() {
            return Ok(stmt);
        }
        let stmt = TYPEINFO_COMPOSITE_QUERY.execute(self).await?.leak();
        self.set_typeinfo_composite(&stmt);
        Ok(stmt)
    }
}

impl Client {
    fn get_enum_variants_blocking(&self, oid: Oid) -> Result<Vec<String>, Error> {
        let stmt = self.typeinfo_enum_statement_blocking()?;
        stmt.bind([oid])
            .query_blocking(self)?
            .into_iter()
            .map(|row| row?.try_get(0))
            .collect()
    }

    fn get_composite_fields_blocking(&self, oid: Oid) -> Result<Vec<Field>, Error> {
        let stmt = self.typeinfo_composite_statement_blocking()?;
        stmt.bind([oid])
            .query_blocking(self)?
            .into_iter()
            .map(|row| {
                let row = row?;
                let name = row.try_get(0)?;
                let oid = row.try_get(1)?;
                let type_ = self._get_type_blocking(oid)?;
                Ok(Field::new(name, type_))
            })
            .collect()
    }

    fn typeinfo_statement_blocking(&self) -> Result<Statement, Error> {
        if let Some(stmt) = self.typeinfo() {
            return Ok(stmt);
        }
        let stmt = match TYPEINFO_QUERY.execute_blocking(self) {
            Ok(stmt) => stmt.leak(),
            Err(e) => {
                return if e
                    .downcast_ref::<DbError>()
                    .is_some_and(|e| SqlState::UNDEFINED_TABLE.eq(e.code()))
                {
                    TYPEINFO_FALLBACK_QUERY.execute_blocking(self).map(|stmt| stmt.leak())
                } else {
                    Err(e)
                }
            }
        };
        self.set_typeinfo(&stmt);
        Ok(stmt)
    }

    fn typeinfo_enum_statement_blocking(&self) -> Result<Statement, Error> {
        if let Some(stmt) = self.typeinfo_enum() {
            return Ok(stmt);
        }
        let stmt = match TYPEINFO_ENUM_QUERY.execute_blocking(self) {
            Ok(stmt) => stmt.leak(),
            Err(e) => {
                return if e
                    .downcast_ref::<DbError>()
                    .is_some_and(|e| SqlState::UNDEFINED_COLUMN.eq(e.code()))
                {
                    TYPEINFO_ENUM_FALLBACK_QUERY
                        .execute_blocking(self)
                        .map(|stmt| stmt.leak())
                } else {
                    Err(e)
                }
            }
        };
        self.set_typeinfo_enum(&stmt);
        Ok(stmt)
    }

    fn typeinfo_composite_statement_blocking(&self) -> Result<Statement, Error> {
        if let Some(stmt) = self.typeinfo_composite() {
            return Ok(stmt);
        }
        let stmt = TYPEINFO_COMPOSITE_QUERY.execute_blocking(self)?.leak();
        self.set_typeinfo_composite(&stmt);
        Ok(stmt)
    }
}
