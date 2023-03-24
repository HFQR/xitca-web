//! Connection configuration. copy/paste from `tokio-postgres`

use core::{fmt, iter, mem, str};

use alloc::borrow::Cow;

use std::{
    error,
    path::{Path, PathBuf},
};

use super::error::Error;

/// Properties required of a session.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TargetSessionAttrs {
    /// No special properties are required.
    Any,
    /// The session must allow writes.
    ReadWrite,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SslMode {
    /// Do not use TLS.
    Disable,
    /// Attempt to connect with TLS but allow sessions without.
    Prefer,
    /// Require the use of TLS.
    Require,
}

/// A host specification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Host {
    /// A TCP hostname.
    Tcp(String),
    Udp(String),
    /// A Unix hostname.
    Unix(PathBuf),
}

#[derive(Clone, Eq, PartialEq)]
pub struct Config {
    pub(crate) user: Option<String>,
    pub(crate) password: Option<Vec<u8>>,
    pub(crate) dbname: Option<String>,
    pub(crate) options: Option<String>,
    pub(crate) application_name: Option<String>,
    pub(crate) ssl_mode: SslMode,
    pub(crate) host: Vec<Host>,
    pub(crate) port: Vec<u16>,
    pub(crate) target_session_attrs: TargetSessionAttrs,
}

impl Default for Config {
    fn default() -> Config {
        Config::new()
    }
}

impl Config {
    /// Creates a new configuration.
    pub const fn new() -> Config {
        Config {
            user: None,
            password: None,
            dbname: None,
            options: None,
            application_name: None,
            ssl_mode: SslMode::Prefer,
            host: Vec::new(),
            port: Vec::new(),
            target_session_attrs: TargetSessionAttrs::Any,
        }
    }

    /// Sets the user to authenticate with.
    ///
    /// Required.
    pub fn user(&mut self, user: &str) -> &mut Config {
        self.user = Some(user.to_string());
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
        self.password = Some(password.as_ref().to_vec());
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
        self.dbname = Some(dbname.to_string());
        self
    }

    /// Gets the name of the database to connect to, if one has been configured
    /// with the `dbname` method.
    pub fn get_dbname(&self) -> Option<&str> {
        self.dbname.as_deref()
    }

    /// Sets command line options used to configure the server.
    pub fn options(&mut self, options: &str) -> &mut Config {
        self.options = Some(options.to_string());
        self
    }

    /// Gets the command line options used to configure the server, if the
    /// options have been set with the `options` method.
    pub fn get_options(&self) -> Option<&str> {
        self.options.as_deref()
    }

    /// Sets the value of the `application_name` runtime parameter.
    pub fn application_name(&mut self, application_name: &str) -> &mut Config {
        self.application_name = Some(application_name.to_string());
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

    pub fn host(&mut self, host: &str) -> &mut Config {
        if host.starts_with('/') {
            return self.host_path(host);
        }

        self.host.push(Host::Tcp(host.to_string()));
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
                    _ => return Err(Error::ToDo),
                };
                self.ssl_mode(mode);
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
                        port.parse().map_err(|_| Error::ToDo)?
                    };
                    self.port(port);
                }
            }
            "target_session_attrs" => {
                let target_session_attrs = match value {
                    "any" => TargetSessionAttrs::Any,
                    "read-write" => TargetSessionAttrs::ReadWrite,
                    _ => {
                        return Err(Error::ToDo);
                    }
                };
                self.target_session_attrs(target_session_attrs);
            }
            _ => {
                return Err(Error::ToDo);
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

#[derive(Debug)]
struct UnknownOption(String);

impl fmt::Display for UnknownOption {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(fmt, "unknown option `{}`", self.0)
    }
}

impl error::Error for UnknownOption {}

#[derive(Debug)]
struct InvalidValue(&'static str);

impl fmt::Display for InvalidValue {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(fmt, "invalid value for option `{}`", self.0)
    }
}

impl error::Error for InvalidValue {}

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
                Err(Error::ToDo)
            }
            None => Err(Error::ToDo),
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

        if s.is_empty() {
            None
        } else {
            Some(s)
        }
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
            return Err(Error::ToDo);
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

        Err(Error::ToDo)
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
                    None => return Err(Error::ToDo),
                };

                let host = &chunk[1..idx];
                let remaining = &chunk[idx + 1..];
                let port = if let Some(port) = remaining.strip_prefix(':') {
                    Some(port)
                } else if remaining.is_empty() {
                    None
                } else {
                    return Err(Error::ToDo);
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
                None => return Err(Error::ToDo),
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
            .map_err(|_| Error::ToDo)
    }
}
