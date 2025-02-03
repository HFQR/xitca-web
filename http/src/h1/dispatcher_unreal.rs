use core::mem::MaybeUninit;

use std::{io, rc::Rc};

use http::StatusCode;
use httparse::{Header, Status};
use xitca_io::{
    bytes::{Buf, BytesMut},
    io::{AsyncIo, Interest},
    net::TcpStream,
};
use xitca_service::Service;
use xitca_unsafe_collection::bytes::read_buf;

use crate::date::{DateTime, DateTimeHandle, DateTimeService};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

pub struct Request<'a, C> {
    pub method: &'a str,
    pub path: &'a str,
    pub headers: &'a mut [Header<'a>],
    pub ctx: &'a C,
}

pub struct Response<'a, const STEP: usize = 1> {
    buf: &'a mut BytesMut,
    date: &'a DateTimeHandle,
}

impl<'a> Response<'a> {
    pub fn status(self, status: StatusCode) -> Response<'a, 2> {
        if status == StatusCode::OK {
            self.buf.extend_from_slice(b"HTTP/1.1 200 OK");
        } else {
            self.buf.extend_from_slice(b"HTTP/1.1 ");
            let reason = status.canonical_reason().unwrap_or("<none>").as_bytes();
            let status = status.as_str().as_bytes();
            self.buf.extend_from_slice(status);
            self.buf.extend_from_slice(b" ");
            self.buf.extend_from_slice(reason);
        }

        Response {
            buf: self.buf,
            date: self.date,
        }
    }
}

impl<'a> Response<'a, 2> {
    pub fn header(self, key: &str, val: &str) -> Self {
        let key = key.as_bytes();
        let val = val.as_bytes();

        self.buf.reserve(key.len() + val.len() + 4);
        self.buf.extend_from_slice(b"\r\n");
        self.buf.extend_from_slice(key);
        self.buf.extend_from_slice(b": ");
        self.buf.extend_from_slice(val);
        self
    }

    pub fn body(self, body: &[u8]) -> Response<'a, 3> {
        super::proto::encode::write_length_header(self.buf, body.len());
        self.body_writer(|buf| buf.extend_from_slice(body))
    }

    pub fn body_writer<F>(mut self, func: F) -> Response<'a, 3>
    where
        F: for<'b> FnOnce(&'b mut BytesMut),
    {
        self.try_write_date();

        self.buf.extend_from_slice(b"\r\n\r\n");

        func(self.buf);

        Response {
            buf: self.buf,
            date: self.date,
        }
    }

    fn try_write_date(&mut self) {
        self.buf.reserve(DateTimeHandle::DATE_VALUE_LENGTH + 12);
        self.buf.extend_from_slice(b"\r\ndate: ");
        self.date.with_date(|date| self.buf.extend_from_slice(date));
    }
}

pub struct Dispatcher<F, C> {
    handler: F,
    ctx: C,
    date: DateTimeService,
}

impl<F, C> Dispatcher<F, C> {
    pub fn new(handler: F, ctx: C) -> Rc<Self> {
        Rc::new(Self {
            handler,
            ctx,
            date: DateTimeService::new(),
        })
    }
}

impl<F, C> Service<TcpStream> for Dispatcher<F, C>
where
    F: for<'h, 'b> AsyncFn(Request<'h, C>, Response<'h>) -> Response<'h, 3>,
{
    type Response = ();
    type Error = Error;

    async fn call(&self, mut stream: TcpStream) -> Result<Self::Response, Self::Error> {
        let mut r_buf = BytesMut::with_capacity(4096);
        let mut w_buf = BytesMut::with_capacity(4096);

        let mut read_closed = false;

        loop {
            stream.ready(Interest::READABLE).await?;

            loop {
                match read_buf(&mut stream, &mut r_buf) {
                    Ok(0) => {
                        if core::mem::replace(&mut read_closed, true) {
                            return Ok(());
                        }
                        break;
                    }
                    Ok(_) => {}
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e.into()),
                }
            }

            loop {
                let mut headers = [const { MaybeUninit::uninit() }; 16];

                let mut req = httparse::Request::new(&mut []);

                match req.parse_with_uninit_headers(r_buf.chunk(), &mut headers)? {
                    Status::Complete(len) => {
                        let req = Request {
                            path: req.path.unwrap(),
                            method: req.method.unwrap(),
                            headers: req.headers,
                            ctx: &self.ctx,
                        };

                        let res = Response {
                            buf: &mut w_buf,
                            date: self.date.get(),
                        };

                        (self.handler)(req, res).await;

                        r_buf.advance(len);
                    }
                    Status::Partial => break,
                };
            }

            let mut written = 0;

            while written != w_buf.len() {
                match io::Write::write(&mut stream, &w_buf[written..]) {
                    Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero).into()),
                    Ok(n) => written += n,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        stream.ready(Interest::WRITABLE).await?;
                    }
                    Err(e) => return Err(e.into()),
                }
            }

            w_buf.clear();
        }
    }
}
