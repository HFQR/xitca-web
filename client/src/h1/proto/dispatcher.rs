use core::{future::poll_fn, pin::Pin};

use std::io;

use futures_core::stream::Stream;
use xitca_http::{body::BodySize, bytes::Buf, h1::proto::codec::TransferCoding};
use xitca_io::io::{AsyncIo, Interest};

use crate::{
    body::BodyError,
    bytes::{Bytes, BytesMut},
    date::DateTimeHandle,
    h1::Error,
    http::{
        header::{HeaderValue, EXPECT, HOST},
        Method, Request, Response, StatusCode,
    },
};

use super::context::Context;

pub(crate) async fn send<S, B, E>(
    stream: &mut S,
    date: DateTimeHandle<'_>,
    req: &mut Request<B>,
) -> Result<(Response<()>, BytesMut, TransferCoding, bool), Error>
where
    S: AsyncIo + Unpin,
    B: Stream<Item = Result<Bytes, E>> + Unpin,
    BodyError: From<E>,
{
    let mut buf = BytesMut::new();

    if !req.headers().contains_key(HOST) {
        if let Some(host) = req.uri().host() {
            buf.reserve(host.len() + 5);
            buf.extend_from_slice(host.as_bytes());

            if let Some(port) = req.uri().port() {
                let port = port.as_str();
                match port {
                    "80" | "443" => {}
                    _ => {
                        buf.extend_from_slice(b":");
                        buf.extend_from_slice(port.as_bytes());
                    }
                }
            }

            let val = HeaderValue::from_maybe_shared(buf.split().freeze()).unwrap();
            req.headers_mut().insert(HOST, val);
        }
    }

    let mut is_expect = req.headers().contains_key(EXPECT);

    if is_expect {
        match BodySize::from_stream(req.body()) {
            // remove expect header if there is no body.
            BodySize::None | BodySize::Sized(0) => {
                let crate::http::header::Entry::Occupied(entry) = req.headers_mut().entry(EXPECT) else {
                    unreachable!()
                };
                entry.remove_entry();
                is_expect = false;
            }
            _ => {}
        }
    }

    let is_tls = req
        .uri()
        .scheme()
        .is_some_and(|scheme| scheme == "https" || scheme == "wss");
    // TODO: make const generic params configurable.
    let mut ctx = Context::<128>::new(&date, is_tls);

    // encode request head and return transfer encoding for request body
    let encoder = ctx.encode_head(&mut buf, req)?;

    // it's important to call set_head_method after encode_head. Context would remove http body it encodes/decodes
    // for head http method.
    if *req.method() == Method::HEAD {
        ctx.set_head_method();
    }

    write_all_buf(stream, &mut buf).await?;

    if is_expect {
        flush(stream).await?;

        loop {
            if let Some((res, mut decoder)) = try_read_response(stream, &mut buf, &mut ctx).await? {
                if res.status() == StatusCode::CONTINUE {
                    break;
                }

                let is_close = ctx.is_connection_closed();

                if ctx.is_head_method() {
                    decoder = TransferCoding::eof();
                }

                return Ok((res, buf, decoder, is_close));
            }
        }
    }

    // TODO: concurrent read write is needed in case server decide to do two way
    // streaming with very large body surpass socket buffer size.
    // (In rare case the server could starting streaming back response without read all the request body)

    // try to send request body.
    if let Err(e) = send_body(stream, encoder, req.body_mut(), &mut buf).await {
        // an error indicate connection should be closed.
        ctx.set_close();
        // clear the buffer as there could be unfinished request data inside.
        buf.clear();

        // we ignore io errors, as the server may want to explain why we cannot write the request body.
        // if this is a connection error it will be handled when we try to read the response.
        // other errors should be propagated as something bad happened and backend may still be waiting for the request body
        // before it can send a response, so it would hang forever if we continue to read the response.
        match e {
            Error::Std(e) => return Err(Error::Std(e)),
            Error::Proto(e) => return Err(Error::Proto(e)),
            Error::Io(_) => (),
        }
    }

    // read response head and get body decoder.
    loop {
        if let Some((res, mut decoder)) = try_read_response(stream, &mut buf, &mut ctx).await? {
            // check if server sent connection close header.

            // *. If send_body function produces error, Context has already set
            // connection type to ConnectionType::CloseForce. We trust the server response
            // to not produce another connection type that override it to any variant
            // other than ConnectionType::Close in this case and only this case.

            let is_close = ctx.is_connection_closed();

            if ctx.is_head_method() {
                decoder = TransferCoding::eof();
            }

            return Ok((res, buf, decoder, is_close));
        }
    }
}

async fn send_body<S, B, E>(
    stream: &mut S,
    mut encoder: TransferCoding,
    body: &mut B,
    buf: &mut BytesMut,
) -> Result<(), Error>
where
    S: AsyncIo,
    B: Stream<Item = Result<Bytes, E>> + Unpin,
    BodyError: From<E>,
{
    if !encoder.is_eof() {
        let mut body = Pin::new(body);

        // poll request body and encode.
        while let Some(bytes) = poll_fn(|cx| body.as_mut().poll_next(cx)).await {
            let bytes = bytes.map_err(BodyError::from)?;
            encoder.encode(bytes, buf);
            // we are not in a hurry here so write before handling next chunk.
            write_all_buf(stream, buf).await?;
        }

        // body is finished. encode eof and clean up.
        encoder.encode_eof(buf);

        write_all_buf(stream, buf).await?;
    }

    flush(stream).await?;

    Ok(())
}

async fn write_all_buf<S>(stream: &mut S, buf: &mut BytesMut) -> io::Result<()>
where
    S: AsyncIo,
{
    while buf.has_remaining() {
        match stream.write(buf.chunk()) {
            Ok(n) => {
                if n == 0 {
                    return Err(io::Error::from(io::ErrorKind::WriteZero));
                }

                buf.advance(n);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                stream.ready(Interest::WRITABLE).await?;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

async fn flush<S>(stream: &mut S) -> io::Result<()>
where
    S: AsyncIo,
{
    while let Err(e) = stream.flush() {
        if e.kind() != io::ErrorKind::WouldBlock {
            return Err(e);
        }
        stream.ready(Interest::WRITABLE).await?;
    }
    Ok(())
}

async fn try_read_response<S>(
    stream: &mut S,
    buf: &mut BytesMut,
    ctx: &mut Context<'_, '_, 128>,
) -> Result<Option<(Response<()>, TransferCoding)>, Error>
where
    S: AsyncIo,
{
    loop {
        match xitca_unsafe_collection::bytes::read_buf(stream, buf) {
            Ok(n) => {
                return if n == 0 {
                    Err(Error::from(io::Error::from(io::ErrorKind::UnexpectedEof)))
                } else {
                    ctx.decode_head(buf).map_err(Into::into)
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                stream.ready(Interest::READABLE).await?;
            }
            Err(e) => return Err(Error::from(e)),
        }
    }
}
