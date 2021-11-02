// use std::cell::RefCell;
//
// use xitca_http::{util::BufList, bytes::Bytes, h1::proto::buf::FlatBuf};
//
// use super::connection::ConnectionWithKey;
//
// pub struct WebSocket<'a> {
//     send_buf: BufList<Bytes>,
//     conn: RefCell<crate::connection::ConnectionWithKey<'a>>,
//     recv_buf: FlatBuf< { 1024 * 1024 }>
// }
