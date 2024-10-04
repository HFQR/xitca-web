use core::mem::MaybeUninit;

use std::rc::Rc;

use http::StatusCode;
use httparse::Status;
use xitca_io::{
    bytes::{Buf, BytesMut},
    net::io_uring::TcpStream,
};
use xitca_service::Service;

pub use httparse::Request;

use crate::date::{DateTime, DateTimeHandle, DateTimeService};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

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
        self.buf.extend_from_slice(b"\r\n");
        self.buf.extend_from_slice(key.as_bytes());
        self.buf.extend_from_slice(b": ");
        self.buf.extend_from_slice(val.as_bytes());
        self
    }

    pub fn body(self, body: &[u8]) -> Response<'a, 3> {
        self.buf.reserve(DateTimeHandle::DATE_VALUE_LENGTH + 12);
        self.buf.extend_from_slice(b"\r\ndate: ");
        self.date.with_date(|date| self.buf.extend_from_slice(date));

        self.buf.extend_from_slice(b"\r\ncontent-length: ");
        self.buf.extend_from_slice(body.len().to_string().as_bytes());
        self.buf.extend_from_slice(b"\r\n\r\n");
        self.buf.extend_from_slice(body);

        Response {
            buf: self.buf,
            date: self.date,
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
    F: for<'h, 'b> xitca_service::AsyncFn<(Request<'h, 'b>, Response<'h>, &'h C), Output = Response<'h, 3>>,
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
                let mut headers = const { [MaybeUninit::uninit(); 8] };

                let mut req = Request::new(&mut []);

                match req.parse_with_uninit_headers(&read_buf, &mut headers)? {
                    Status::Complete(len) => {
                        let res = Response {
                            buf: &mut write_buf,
                            date: self.date.get(),
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
