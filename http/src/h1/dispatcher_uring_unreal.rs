use core::mem::MaybeUninit;

use std::rc::Rc;

use http::StatusCode;
use httparse::{Header, Status};
use xitca_io::{
    bytes::{Buf, BytesMut},
    net::io_uring::TcpStream,
};
use xitca_service::{AsyncFn, Service};

use crate::date::{DateTime, DateTimeHandle, DateTimeService};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

pub struct Request<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub headers: &'a mut [Header<'a>],
}

pub struct Response<'a, const STEP: usize = 1> {
    buf: &'a mut BytesMut,
    date: &'a DateTimeHandle,
    want_write_date: bool,
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
            want_write_date: self.want_write_date,
        }
    }
}

impl<'a> Response<'a, 2> {
    pub fn header(mut self, key: &str, val: &str) -> Self {
        if key.eq_ignore_ascii_case("date") {
            self.want_write_date = false;
        }

        let key = key.as_bytes();
        let val = val.as_bytes();

        self.buf.reserve(key.len() + val.len() + 4);
        self.buf.extend_from_slice(b"\r\n");
        self.buf.extend_from_slice(key);
        self.buf.extend_from_slice(b": ");
        self.buf.extend_from_slice(val);
        self
    }

    pub fn body(mut self, body: &[u8]) -> Response<'a, 3> {
        self.try_write_date();

        super::proto::encode::write_length_header(self.buf, body.len());
        self.buf.extend_from_slice(b"\r\n\r\n");
        self.buf.extend_from_slice(body);

        Response {
            buf: self.buf,
            date: self.date,
            want_write_date: self.want_write_date,
        }
    }

    pub fn body_writer<F, E>(mut self, func: F) -> Result<Response<'a, 3>, E>
    where
        F: for<'b> FnOnce(&'b mut BytesMut) -> Result<(), E>,
    {
        self.try_write_date();

        self.buf.extend_from_slice(b"\r\n\r\n");

        func(self.buf)?;

        Ok(Response {
            buf: self.buf,
            date: self.date,
            want_write_date: self.want_write_date,
        })
    }

    fn try_write_date(&mut self) {
        if self.want_write_date {
            self.buf.reserve(DateTimeHandle::DATE_VALUE_LENGTH + 12);
            self.buf.extend_from_slice(b"\r\ndate: ");
            self.date.with_date(|date| self.buf.extend_from_slice(date));
        }
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
    F: for<'h, 'b> AsyncFn<(Request<'h>, Response<'h>, &'h C), Output = Response<'h, 3>>,
{
    type Response = ();
    type Error = Error;

    async fn call(&self, stream: TcpStream) -> Result<Self::Response, Self::Error> {
        let mut read_buf = BytesMut::with_capacity(4096);
        let mut write_buf = BytesMut::with_capacity(4096);

        loop {
            read_buf.reserve(4096);

            let (res, buf) = stream.read(read_buf).await;

            if res? == 0 {
                return Ok(());
            }

            read_buf = buf;

            'inner: loop {
                let mut headers = [const { MaybeUninit::uninit() }; 8];

                let mut req = httparse::Request::new(&mut []);

                match req.parse_with_uninit_headers(&read_buf, &mut headers)? {
                    Status::Complete(len) => {
                        let req = Request {
                            path: req.path.unwrap(),
                            method: req.method.unwrap(),
                            headers: req.headers,
                        };

                        let res = Response {
                            buf: &mut write_buf,
                            date: self.date.get(),
                            want_write_date: true,
                        };

                        self.handler.call((req, res, &self.ctx)).await;

                        read_buf.advance(len);
                    }
                    Status::Partial => break 'inner,
                };
            }

            let (res, buf) = stream.write_all(write_buf).await;
            res?;
            write_buf = buf;
            write_buf.clear();
        }
    }
}
