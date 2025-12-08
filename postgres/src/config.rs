//! Connection configuration. copy/paste from `tokio-postgres`

use core::{fmt, iter, mem, str};

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use super::{error::Error, session::TargetSessionAttrs};

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SslMode {
    /// Do not use TLS.
    #[cfg_attr(not(feature = "tls"), default)]
    Disable,
    /// Attempt to connect with TLS but allow sessions without.
    #[cfg_attr(feature = "tls", default)]
    Prefer,
    /// Require the use of TLS.
    Require,
}

/// TLS negotiation configuration
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SslNegotiation {
    /// Use PostgreSQL SslRequest for Ssl negotiation
    #[default]
    Postgres,
    /// Start Ssl handshake without negotiation, only works for PostgreSQL 17+
    Direct,
}

/// A host specification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Host {
    /// A TCP hostname.
    Tcp(Box<str>),
    Quic(Box<str>),
    /// A Unix hostname.
    Unix(PathBuf),
}

#[derive(Clone, Eq, PartialEq)]
pub struct Config {
    pub(crate) user: Option<Box<str>>,
    pub(crate) password: Option<Box<[u8]>>,
    pub(crate) dbname: Option<Box<str>>,
    pub(crate) options: Option<Box<str>>,
    pub(crate) application_name: Option<Box<str>>,
    pub(crate) ssl_mode: SslMode,
    pub(crate) ssl_negotiation: SslNegotiation,
    pub(crate) host: Vec<Host>,
    pub(crate) port: Vec<u16>,
    target_session_attrs: TargetSessionAttrs,
    tls_server_end_point: Option<Box<[u8]>>,
}

impl Default for Config {
    fn default() -> Config {
        Config::new()
    }
}

impl Config {
    /// Creates a new configuration.
    pub fn new() -> Config {
        Config {
            user: None,
            password: None,
            dbname: None,
            options: None,
            application_name: None,
            ssl_mode: SslMode::default(),
            ssl_negotiation: SslNegotiation::Postgres,
            host: Vec::new(),
            port: Vec::new(),
            target_session_attrs: TargetSessionAttrs::Any,
            tls_server_end_point: None,
        }
    }

    /// Sets the user to authenticate with.
    ///
    /// Required.
    pub fn user(&mut self, user: &str) -> &mut Config {
        self.user = Some(Box::from(user));
        self
    }

    /// Gets the user to authenticate with, if one has been configured with
    /// the `user` method.
    pub fn get_user(&self) -> Option<&str> {
        self.user.as_deref()
    }

    /// Sets the password to authenticate with.
    pub fn password<T>(&mut self, password: T) -> &mut Config
    where
        T: AsRef<[u8]>,
    {
        self.password = Some(Box::from(password.as_ref()));
        self
    }

    /// Gets the password to authenticate with, if one has been configured with
    /// the `password` method.
    pub fn get_password(&self) -> Option<&[u8]> {
        self.password.as_deref()
    }

    /// Sets the name of the database to connect to.
    ///
    /// Defaults to the user.
    pub fn dbname(&mut self, dbname: &str) -> &mut Config {
        self.dbname = Some(Box::from(dbname));
        self
    }

    /// Gets the name of the database to connect to, if one has been configured
    /// with the `dbname` method.
    pub fn get_dbname(&self) -> Option<&str> {
        self.dbname.as_deref()
    }

    /// Sets command line options used to configure the server.
    pub fn options(&mut self, options: &str) -> &mut Config {
        self.options = Some(Box::from(options));
        self
    }

    /// Gets the command line options used to configure the server, if the
    /// options have been set with the `options` method.
    pub fn get_options(&self) -> Option<&str> {
        self.options.as_deref()
    }

    /// Sets the value of the `application_name` runtime parameter.
    pub fn application_name(&mut self, application_name: &str) -> &mut Config {
        self.application_name = Some(Box::from(application_name));
        self
    }

    /// Gets the value of the `application_name` runtime parameter, if it has
    /// been set with the `application_name` method.
    pub fn get_application_name(&self) -> Option<&str> {
        self.application_name.as_deref()
    }

    /// Sets the SSL configuration.
    ///
    /// Defaults to `prefer`.
    pub fn ssl_mode(&mut self, ssl_mode: SslMode) -> &mut Config {
        self.ssl_mode = ssl_mode;
        self
    }

    /// Gets the SSL configuration.
    pub fn get_ssl_mode(&self) -> SslMode {
        self.ssl_mode
    }

    /// Sets the SSL negotiation method.
    ///
    /// Defaults to `postgres`.
    pub fn ssl_negotiation(&mut self, ssl_negotiation: SslNegotiation) -> &mut Config {
        self.ssl_negotiation = ssl_negotiation;
        self
    }

    /// Gets the SSL negotiation method.
    pub fn get_ssl_negotiation(&self) -> SslNegotiation {
        self.ssl_negotiation
    }

    pub fn host(&mut self, host: &str) -> &mut Config {
        if host.starts_with('/') {
            return self.host_path(host);
        }

        let host = Host::Tcp(Box::from(host));

        self.host.push(host);
        self
    }

    /// Adds a Unix socket host to the configuration.
    ///
    /// Unlike `host`, this method allows non-UTF8 paths.
    pub fn host_path<T>(&mut self, host: T) -> &mut Config
    where
        T: AsRef<Path>,
    {
        self.host.push(Host::Unix(host.as_ref().to_path_buf()));
        self
    }

    /// Gets the hosts that have been added to the configuration with `host`.
    pub fn get_hosts(&self) -> &[Host] {
        &self.host
    }

    /// Adds a port to the configuration.
    ///
    /// Multiple ports can be specified by calling this method multiple times. There must either be no ports, in which
    /// case the default of 5432 is used, a single port, in which it is used for all hosts, or the same number of ports
    /// as hosts.
    pub fn port(&mut self, port: u16) -> &mut Config {
        self.port.push(port);
        self
    }

    /// Gets the ports that have been added to the configuration with `port`.
    pub fn get_ports(&self) -> &[u16] {
        &self.port
    }

    /// Sets the requirements of the session.
    ///
    /// This can be used to connect to the primary server in a clustered database rather than one of the read-only
    /// secondary servers. Defaults to `Any`.
    pub fn target_session_attrs(&mut self, target_session_attrs: TargetSessionAttrs) -> &mut Config {
        self.target_session_attrs = target_session_attrs;
        self
    }

    /// Gets the requirements of the session.
    pub fn get_target_session_attrs(&self) -> TargetSessionAttrs {
        self.target_session_attrs
    }

    /// change the remote peer's tls certificates. it's often coupled with [`Postgres::connect_io`] API for manual tls
    /// session connecting and channel binding authentication.
    /// # Examples
    /// ```rust
    /// use xitca_postgres::{Config, Postgres};
    ///
    /// // handle tls connection on your own.
    /// async fn connect_io() {
    ///     let mut cfg = Config::try_from("postgres://postgres:postgres@localhost/postgres").unwrap();
    ///     
    ///     // an imaginary function where you establish a tls connection to database on your own.
    ///     // the established connection should be providing valid cert bytes.
    ///     let (io, certs) = your_tls_connector().await;
    ///
    ///     // set cert bytes to configuration
    ///     cfg.tls_server_end_point(certs);
    ///
    ///     // give xitca-postgres the config and established io and finish db session process.
    ///     let _ = Postgres::new(cfg).connect_io(io).await;
    /// }
    ///
    /// async fn your_tls_connector() -> (MyTlsStream, Vec<u8>) {
    ///     todo!("your tls connecting logic lives here. the process can be async or not.")
    /// }
    ///
    /// // a possible type representation of your manual tls connection to database
    /// struct MyTlsStream;
    ///
    /// # use std::{io, pin::Pin, task::{Context, Poll}};
    /// #
    /// # use xitca_io::io::{AsyncIo, Interest, Ready};
    /// #   
    /// # impl AsyncIo for MyTlsStream {
    /// #   async fn ready(&mut self, interest: Interest) -> io::Result<Ready> {
    /// #       todo!()
    /// #   }
    /// #
    /// #   fn poll_ready(&mut self, interest: Interest, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
    /// #       todo!()
    /// #   }
    /// #   
    /// #   fn is_vectored_write(&self) -> bool {
    /// #       false
    /// #   }
    /// #   
    /// #   fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
    /// #       Poll::Ready(Ok(()))
    /// #   }
    /// # }
    /// #   
    /// # impl io::Read for MyTlsStream {
    /// #   fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
    /// #       todo!()
    /// #   }
    /// # }   
    /// #
    /// # impl io::Write for MyTlsStream {
    /// #   fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
    /// #       todo!()
    /// #   }
    /// #   
    /// #   fn flush(&mut self) -> io::Result<()> {
    /// #       Ok(())
    /// #   }
    /// # }
    /// ```
    ///
    /// [`Postgres::connect_io`]: crate::Postgres::connect_io
    pub fn tls_server_end_point(&mut self, tls_server_end_point: impl AsRef<[u8]>) -> &mut Self {
        self.tls_server_end_point = Some(Box::from(tls_server_end_point.as_ref()));
        self
    }

    pub fn get_tls_server_end_point(&self) -> Option<&[u8]> {
        self.tls_server_end_point.as_deref()
    }

    fn param(&mut self, key: &str, value: &str) -> Result<(), Error> {
        match key {
            "user" => {
                self.user(value);
            }
            "password" => {
                self.password(value);
            }
            "dbname" => {
                self.dbname(value);
            }
            "options" => {
                self.options(value);
            }
            "application_name" => {
                self.application_name(value);
            }
            "sslmode" => {
                let mode = match value {
                    "disable" => SslMode::Disable,
                    "prefer" => SslMode::Prefer,
                    "require" => SslMode::Require,
                    _ => return Err(Error::todo()),
                };
                self.ssl_mode(mode);
            }
            "sslnegotiation" => {
                let mode = match value {
                    "postgres" => SslNegotiation::Postgres,
                    "direct" => SslNegotiation::Direct,
                    _ => return Err(Error::todo()),
                };
                self.ssl_negotiation(mode);
            }
            "host" => {
                for host in value.split(',') {
                    self.host(host);
                }
            }
            "port" => {
                for port in value.split(',') {
                    let port = if port.is_empty() {
                        5432
                    } else {
                        port.parse().map_err(|_| Error::todo())?
                    };
                    self.port(port);
                }
            }
            "target_session_attrs" => {
                let target_session_attrs = match value {
                    "any" => TargetSessionAttrs::Any,
                    "read-write" => TargetSessionAttrs::ReadWrite,
                    "read-only" => TargetSessionAttrs::ReadOnly,
                    _ => return Err(Error::todo()),
                };
                self.target_session_attrs(target_session_attrs);
            }
            _ => {
                return Err(Error::todo());
            }
        }

        Ok(())
    }
}

impl TryFrom<String> for Config {
    type Error = Error;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::try_from(s.as_str())
    }
}

impl TryFrom<&str> for Config {
    type Error = Error;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match UrlParser::parse(s)? {
            Some(config) => Ok(config),
            None => Parser::parse(s),
        }
    }
}

// Omit password from debug output
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct Redaction {}
        impl fmt::Debug for Redaction {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "_")
            }
        }

        f.debug_struct("Config")
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| Redaction {}))
            .field("dbname", &self.dbname)
            .field("options", &self.options)
            .field("application_name", &self.application_name)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("target_session_attrs", &self.target_session_attrs)
            .finish()
    }
}

struct Parser<'a> {
    s: &'a str,
    it: iter::Peekable<str::CharIndices<'a>>,
}

impl<'a> Parser<'a> {
    fn parse(s: &'a str) -> Result<Config, Error> {
        let mut parser = Parser {
            s,
            it: s.char_indices().peekable(),
        };

        let mut config = Config::new();

        while let Some((key, value)) = parser.parameter()? {
            config.param(key, &value)?;
        }

        Ok(config)
    }

    fn skip_ws(&mut self) {
        self.take_while(char::is_whitespace);
    }

    fn take_while<F>(&mut self, f: F) -> &'a str
    where
        F: Fn(char) -> bool,
    {
        let start = match self.it.peek() {
            Some(&(i, _)) => i,
            None => return "",
        };

        loop {
            match self.it.peek() {
                Some(&(_, c)) if f(c) => {
                    self.it.next();
                }
                Some(&(i, _)) => return &self.s[start..i],
                None => return &self.s[start..],
            }
        }
    }

    fn eat(&mut self, target: char) -> Result<(), Error> {
        match self.it.next() {
            Some((_, c)) if c == target => Ok(()),
            Some((i, c)) => {
                let _m = format!("unexpected character at byte {i}: expected `{target}` but got `{c}`");
                Err(Error::todo())
            }
            None => Err(Error::todo()),
        }
    }

    fn eat_if(&mut self, target: char) -> bool {
        match self.it.peek() {
            Some(&(_, c)) if c == target => {
                self.it.next();
                true
            }
            _ => false,
        }
    }

    fn keyword(&mut self) -> Option<&'a str> {
        let s = self.take_while(|c| match c {
            c if c.is_whitespace() => false,
            '=' => false,
            _ => true,
        });

        if s.is_empty() { None } else { Some(s) }
    }

    fn value(&mut self) -> Result<String, Error> {
        let value = if self.eat_if('\'') {
            let value = self.quoted_value()?;
            self.eat('\'')?;
            value
        } else {
            self.simple_value()?
        };

        Ok(value)
    }

    fn simple_value(&mut self) -> Result<String, Error> {
        let mut value = String::new();

        while let Some(&(_, c)) = self.it.peek() {
            if c.is_whitespace() {
                break;
            }

            self.it.next();
            if c == '\\' {
                if let Some((_, c2)) = self.it.next() {
                    value.push(c2);
                }
            } else {
                value.push(c);
            }
        }

        if value.is_empty() {
            return Err(Error::todo());
        }

        Ok(value)
    }

    fn quoted_value(&mut self) -> Result<String, Error> {
        let mut value = String::new();

        while let Some(&(_, c)) = self.it.peek() {
            if c == '\'' {
                return Ok(value);
            }

            self.it.next();
            if c == '\\' {
                if let Some((_, c2)) = self.it.next() {
                    value.push(c2);
                }
            } else {
                value.push(c);
            }
        }

        Err(Error::todo())
    }

    fn parameter(&mut self) -> Result<Option<(&'a str, String)>, Error> {
        self.skip_ws();
        let keyword = match self.keyword() {
            Some(keyword) => keyword,
            None => return Ok(None),
        };
        self.skip_ws();
        self.eat('=')?;
        self.skip_ws();
        let value = self.value()?;

        Ok(Some((keyword, value)))
    }
}

// This is a pretty sloppy "URL" parser, but it matches the behavior of libpq, where things really aren't very strict
struct UrlParser<'a> {
    s: &'a str,
    config: Config,
}

impl<'a> UrlParser<'a> {
    fn parse(s: &'a str) -> Result<Option<Config>, Error> {
        let s = match Self::remove_url_prefix(s) {
            Some(s) => s,
            None => return Ok(None),
        };

        let mut parser = UrlParser {
            s,
            config: Config::new(),
        };

        parser.parse_credentials()?;
        parser.parse_host()?;
        parser.parse_path()?;
        parser.parse_params()?;

        Ok(Some(parser.config))
    }

    fn remove_url_prefix(s: &str) -> Option<&str> {
        for prefix in &["postgres://", "postgresql://"] {
            if let Some(stripped) = s.strip_prefix(prefix) {
                return Some(stripped);
            }
        }

        None
    }

    fn take_until(&mut self, end: &[char]) -> Option<&'a str> {
        match self.s.find(end) {
            Some(pos) => {
                let (head, tail) = self.s.split_at(pos);
                self.s = tail;
                Some(head)
            }
            None => None,
        }
    }

    fn take_all(&mut self) -> &'a str {
        mem::take(&mut self.s)
    }

    fn eat_byte(&mut self) {
        self.s = &self.s[1..];
    }

    fn parse_credentials(&mut self) -> Result<(), Error> {
        let creds = match self.take_until(&['@']) {
            Some(creds) => creds,
            None => return Ok(()),
        };
        self.eat_byte();

        let mut it = creds.splitn(2, ':');
        let user = self.decode(it.next().unwrap())?;
        self.config.user(&user);

        if let Some(password) = it.next() {
            let password = Cow::from(percent_encoding::percent_decode(password.as_bytes()));
            self.config.password(password);
        }

        Ok(())
    }

    fn parse_host(&mut self) -> Result<(), Error> {
        let host = match self.take_until(&['/', '?']) {
            Some(host) => host,
            None => self.take_all(),
        };

        if host.is_empty() {
            return Ok(());
        }

        for chunk in host.split(',') {
            let (host, port) = if chunk.starts_with('[') {
                let idx = match chunk.find(']') {
                    Some(idx) => idx,
                    None => return Err(Error::todo()),
                };

                let host = &chunk[1..idx];
                let remaining = &chunk[idx + 1..];
                let port = if let Some(port) = remaining.strip_prefix(':') {
                    Some(port)
                } else if remaining.is_empty() {
                    None
                } else {
                    return Err(Error::todo());
                };

                (host, port)
            } else {
                let mut it = chunk.splitn(2, ':');
                (it.next().unwrap(), it.next())
            };

            self.host_param(host)?;
            let port = self.decode(port.unwrap_or("5432"))?;
            self.config.param("port", &port)?;
        }

        Ok(())
    }

    fn parse_path(&mut self) -> Result<(), Error> {
        if !self.s.starts_with('/') {
            return Ok(());
        }
        self.eat_byte();

        let dbname = match self.take_until(&['?']) {
            Some(dbname) => dbname,
            None => self.take_all(),
        };

        if !dbname.is_empty() {
            self.config.dbname(&self.decode(dbname)?);
        }

        Ok(())
    }

    fn parse_params(&mut self) -> Result<(), Error> {
        if !self.s.starts_with('?') {
            return Ok(());
        }
        self.eat_byte();

        while !self.s.is_empty() {
            let key = match self.take_until(&['=']) {
                Some(key) => self.decode(key)?,
                None => return Err(Error::todo()),
            };
            self.eat_byte();

            let value = match self.take_until(&['&']) {
                Some(value) => {
                    self.eat_byte();
                    value
                }
                None => self.take_all(),
            };

            if key == "host" {
                self.host_param(value)?;
            } else {
                let value = self.decode(value)?;
                self.config.param(&key, &value)?;
            }
        }

        Ok(())
    }

    fn host_param(&mut self, s: &str) -> Result<(), Error> {
        let s = self.decode(s)?;
        self.config.param("host", &s)
    }

    fn decode(&self, s: &'a str) -> Result<Cow<'a, str>, Error> {
        percent_encoding::percent_decode(s.as_bytes())
            .decode_utf8()
            .map_err(|_| Error::todo())
    }
}
