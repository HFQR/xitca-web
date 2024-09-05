//! library error types with re-export error from `rust-postgres`

mod sql_state;

pub use postgres_types::{WasNull, WrongType};

use core::{
    convert::Infallible,
    fmt,
    ops::{Deref, DerefMut},
};

use std::{error, io};

use fallible_iterator::FallibleIterator;
use postgres_protocol::message::backend::ErrorFields;

use super::from_sql::FromSqlError;

use self::sql_state::SqlState;

/// public facing error type. providing basic format and display based error handling.
///
/// for typed based error handling runtime type cast is needed with the help of other
/// public error types offered by this module.
///
/// # Example
/// ```rust
/// use xitca_postgres::error::{DriverDown, Error};
///
/// fn is_driver_down(e: Error) -> bool {
///     // downcast error to DriverDown error type to check if client driver is gone.
///     e.downcast_ref::<DriverDown>().is_some()
/// }
/// ```
pub struct Error(Box<dyn error::Error + Send + Sync>);

impl Error {
    pub fn is_driver_down(&self) -> bool {
        self.0.is::<DriverDown>() || self.0.is::<DriverDownReceiving>()
    }

    pub(crate) fn todo() -> Self {
        Self("WIP error type placeholder".to_string().into())
    }

    pub(crate) fn unexpected() -> Self {
        Self(Box::new(UnexpectedMessage))
    }
}

impl Deref for Error {
    type Target = dyn error::Error + Send + Sync;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl DerefMut for Error {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.0
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.0.source()
    }
}

/// error indicate [Client]'s [Driver] is dropped and can't be accessed anymore when sending request to driver.
///
/// database query related to this error has not been sent to database and it's safe to retry operation if
/// desired.
///
/// [Client]: crate::client::Client
/// [Driver]: crate::driver::Driver
#[derive(Default)]
pub struct DriverDown;

impl fmt::Debug for DriverDown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DriverDown").finish()
    }
}

impl fmt::Display for DriverDown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Driver is dropped and unaccessible.")
    }
}

impl error::Error for DriverDown {}

impl From<DriverDown> for Error {
    fn from(e: DriverDown) -> Self {
        Self(Box::new(e))
    }
}

/// error indicate [Client]'s [Driver] is dropped and can't be accessed anymore when receiving response
/// from server.
///
/// all mid flight response and unfinished response data are lost and can't be recovered. database query
/// related to this error may or may not executed successfully and it should not be retried blindly.
///
/// [Client]: crate::client::Client
/// [Driver]: crate::driver::Driver
#[derive(Debug)]
pub struct DriverDownReceiving;

impl fmt::Display for DriverDownReceiving {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Driver is dropped and unaccessible. Response data is lost and unrecoverable.")
    }
}

impl error::Error for DriverDownReceiving {}

impl From<DriverDownReceiving> for Error {
    fn from(e: DriverDownReceiving) -> Self {
        Self(Box::new(e))
    }
}

pub struct InvalidColumnIndex(pub String);

impl fmt::Debug for InvalidColumnIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InvalidColumnIndex").finish()
    }
}

impl fmt::Display for InvalidColumnIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid column index: {}", self.0)
    }
}

impl error::Error for InvalidColumnIndex {}

impl From<InvalidColumnIndex> for Error {
    fn from(e: InvalidColumnIndex) -> Self {
        Self(Box::new(e))
    }
}

impl From<Infallible> for Error {
    fn from(e: Infallible) -> Self {
        match e {}
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self(Box::new(e))
    }
}

impl From<FromSqlError> for Error {
    fn from(e: FromSqlError) -> Self {
        Self(e)
    }
}

/// error happens when library user failed to provide valid authentication info to database server.
#[derive(Debug)]
pub enum AuthenticationError {
    MissingUserName,
    MissingPassWord,
    WrongPassWord,
}

impl fmt::Display for AuthenticationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MissingUserName => f.write_str("username is missing")?,
            Self::MissingPassWord => f.write_str("password is missing")?,
            Self::WrongPassWord => f.write_str("password is wrong")?,
        }
        f.write_str(" for authentication")
    }
}

impl error::Error for AuthenticationError {}

impl From<AuthenticationError> for Error {
    fn from(e: AuthenticationError) -> Self {
        Self(Box::new(e))
    }
}

#[non_exhaustive]
#[derive(Debug)]
pub enum SystemError {
    Unix,
}

impl fmt::Display for SystemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Unix => f.write_str("unix")?,
        }
        f.write_str(" system is not available")
    }
}

impl error::Error for SystemError {}

impl From<SystemError> for Error {
    fn from(e: SystemError) -> Self {
        Self(Box::new(e))
    }
}

#[non_exhaustive]
#[derive(Debug)]
pub enum FeatureError {
    Tls,
    Quic,
}

impl fmt::Display for FeatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Tls => f.write_str("tls")?,
            Self::Quic => f.write_str("quic")?,
        }
        f.write_str(" feature is not enabled")
    }
}

impl error::Error for FeatureError {}

impl From<FeatureError> for Error {
    fn from(e: FeatureError) -> Self {
        Self(Box::new(e))
    }
}

#[derive(Debug)]
pub struct UnexpectedMessage;

impl fmt::Display for UnexpectedMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unexpected message from database")
    }
}

impl error::Error for UnexpectedMessage {}

#[cold]
#[inline(never)]
pub(crate) fn unexpected_eof_err() -> io::Error {
    io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "zero byte read. remote close connection unexpectedly",
    )
}

impl From<WrongType> for Error {
    fn from(e: WrongType) -> Self {
        Self(Box::new(e))
    }
}

/// A Postgres error or notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbError {
    severity: String,
    parsed_severity: Option<Severity>,
    code: SqlState,
    message: String,
    detail: Option<String>,
    hint: Option<String>,
    position: Option<ErrorPosition>,
    where_: Option<String>,
    schema: Option<String>,
    table: Option<String>,
    column: Option<String>,
    datatype: Option<String>,
    constraint: Option<String>,
    file: Option<String>,
    line: Option<u32>,
    routine: Option<String>,
}

impl DbError {
    #[cold]
    #[inline(never)]
    pub(crate) fn parse(fields: &mut ErrorFields<'_>) -> io::Result<DbError> {
        let mut severity = None;
        let mut parsed_severity = None;
        let mut code = None;
        let mut message = None;
        let mut detail = None;
        let mut hint = None;
        let mut normal_position = None;
        let mut internal_position = None;
        let mut internal_query = None;
        let mut where_ = None;
        let mut schema = None;
        let mut table = None;
        let mut column = None;
        let mut datatype = None;
        let mut constraint = None;
        let mut file = None;
        let mut line = None;
        let mut routine = None;

        while let Some(field) = fields.next()? {
            let value = String::from_utf8_lossy(field.value_bytes());
            match field.type_() {
                b'S' => severity = Some(value.into_owned()),
                b'C' => code = Some(SqlState::from_code(&value)),
                b'M' => message = Some(value.into_owned()),
                b'D' => detail = Some(value.into_owned()),
                b'H' => hint = Some(value.into_owned()),
                b'P' => {
                    normal_position = Some(value.parse::<u32>().map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidInput, "`P` field did not contain an integer")
                    })?);
                }
                b'p' => {
                    internal_position = Some(value.parse::<u32>().map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidInput, "`p` field did not contain an integer")
                    })?);
                }
                b'q' => internal_query = Some(value.into_owned()),
                b'W' => where_ = Some(value.into_owned()),
                b's' => schema = Some(value.into_owned()),
                b't' => table = Some(value.into_owned()),
                b'c' => column = Some(value.into_owned()),
                b'd' => datatype = Some(value.into_owned()),
                b'n' => constraint = Some(value.into_owned()),
                b'F' => file = Some(value.into_owned()),
                b'L' => {
                    line = Some(value.parse::<u32>().map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidInput, "`L` field did not contain an integer")
                    })?);
                }
                b'R' => routine = Some(value.into_owned()),
                b'V' => {
                    parsed_severity = Some(Severity::from_str(&value).ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "`V` field contained an invalid value")
                    })?);
                }
                _ => {}
            }
        }

        Ok(DbError {
            severity: severity.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "`S` field missing"))?,
            parsed_severity,
            code: code.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "`C` field missing"))?,
            message: message.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "`M` field missing"))?,
            detail,
            hint,
            position: match normal_position {
                Some(position) => Some(ErrorPosition::Original(position)),
                None => match internal_position {
                    Some(position) => Some(ErrorPosition::Internal {
                        position,
                        query: internal_query.ok_or_else(|| {
                            io::Error::new(io::ErrorKind::InvalidInput, "`q` field missing but `p` field present")
                        })?,
                    }),
                    None => None,
                },
            },
            where_,
            schema,
            table,
            column,
            datatype,
            constraint,
            file,
            line,
            routine,
        })
    }

    /// The field contents are ERROR, FATAL, or PANIC (in an error message),
    /// or WARNING, NOTICE, DEBUG, INFO, or LOG (in a notice message), or a
    /// localized translation of one of these.
    pub fn severity(&self) -> &str {
        &self.severity
    }

    /// A parsed, nonlocalized version of `severity`. (PostgreSQL 9.6+)
    pub fn parsed_severity(&self) -> Option<Severity> {
        self.parsed_severity
    }

    /// The SQLSTATE code for the error.
    pub fn code(&self) -> &SqlState {
        &self.code
    }

    /// The primary human-readable error message.
    ///
    /// This should be accurate but terse (typically one line).
    pub fn message(&self) -> &str {
        &self.message
    }

    /// An optional secondary error message carrying more detail about the
    /// problem.
    ///
    /// Might run to multiple lines.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// An optional suggestion what to do about the problem.
    ///
    /// This is intended to differ from `detail` in that it offers advice
    /// (potentially inappropriate) rather than hard facts. Might run to
    /// multiple lines.
    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }

    /// An optional error cursor position into either the original query string
    /// or an internally generated query.
    pub fn position(&self) -> Option<&ErrorPosition> {
        self.position.as_ref()
    }

    /// An indication of the context in which the error occurred.
    ///
    /// Presently this includes a call stack traceback of active procedural
    /// language functions and internally-generated queries. The trace is one
    /// entry per line, most recent first.
    pub fn where_(&self) -> Option<&str> {
        self.where_.as_deref()
    }

    /// If the error was associated with a specific database object, the name
    /// of the schema containing that object, if any. (PostgreSQL 9.3+)
    pub fn schema(&self) -> Option<&str> {
        self.schema.as_deref()
    }

    /// If the error was associated with a specific table, the name of the
    /// table. (Refer to the schema name field for the name of the table's
    /// schema.) (PostgreSQL 9.3+)
    pub fn table(&self) -> Option<&str> {
        self.table.as_deref()
    }

    /// If the error was associated with a specific table column, the name of
    /// the column.
    ///
    /// (Refer to the schema and table name fields to identify the table.)
    /// (PostgreSQL 9.3+)
    pub fn column(&self) -> Option<&str> {
        self.column.as_deref()
    }

    /// If the error was associated with a specific data type, the name of the
    /// data type. (Refer to the schema name field for the name of the data
    /// type's schema.) (PostgreSQL 9.3+)
    pub fn datatype(&self) -> Option<&str> {
        self.datatype.as_deref()
    }

    /// If the error was associated with a specific constraint, the name of the
    /// constraint.
    ///
    /// Refer to fields listed above for the associated table or domain.
    /// (For this purpose, indexes are treated as constraints, even if they
    /// weren't created with constraint syntax.) (PostgreSQL 9.3+)
    pub fn constraint(&self) -> Option<&str> {
        self.constraint.as_deref()
    }

    /// The file name of the source-code location where the error was reported.
    pub fn file(&self) -> Option<&str> {
        self.file.as_deref()
    }

    /// The line number of the source-code location where the error was
    /// reported.
    pub fn line(&self) -> Option<u32> {
        self.line
    }

    /// The name of the source-code routine reporting the error.
    pub fn routine(&self) -> Option<&str> {
        self.routine.as_deref()
    }
}

impl fmt::Display for DbError {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(fmt, "{}: {}", self.severity, self.message)?;
        if let Some(detail) = &self.detail {
            write!(fmt, "\nDETAIL: {}", detail)?;
        }
        if let Some(hint) = &self.hint {
            write!(fmt, "\nHINT: {}", hint)?;
        }
        Ok(())
    }
}

impl error::Error for DbError {}

impl From<DbError> for Error {
    fn from(e: DbError) -> Self {
        Self(Box::new(e))
    }
}

/// The severity of a Postgres error or notice.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Severity {
    /// PANIC
    Panic,
    /// FATAL
    Fatal,
    /// ERROR
    Error,
    /// WARNING
    Warning,
    /// NOTICE
    Notice,
    /// DEBUG
    Debug,
    /// INFO
    Info,
    /// LOG
    Log,
}

impl fmt::Display for Severity {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match *self {
            Severity::Panic => "PANIC",
            Severity::Fatal => "FATAL",
            Severity::Error => "ERROR",
            Severity::Warning => "WARNING",
            Severity::Notice => "NOTICE",
            Severity::Debug => "DEBUG",
            Severity::Info => "INFO",
            Severity::Log => "LOG",
        };
        fmt.write_str(s)
    }
}

impl Severity {
    fn from_str(s: &str) -> Option<Severity> {
        match s {
            "PANIC" => Some(Severity::Panic),
            "FATAL" => Some(Severity::Fatal),
            "ERROR" => Some(Severity::Error),
            "WARNING" => Some(Severity::Warning),
            "NOTICE" => Some(Severity::Notice),
            "DEBUG" => Some(Severity::Debug),
            "INFO" => Some(Severity::Info),
            "LOG" => Some(Severity::Log),
            _ => None,
        }
    }
}

/// Represents the position of an error in a query.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ErrorPosition {
    /// A position in the original query.
    Original(u32),
    /// A position in an internally generated query.
    Internal {
        /// The byte position.
        position: u32,
        /// A query generated by the Postgres server.
        query: String,
    },
}
