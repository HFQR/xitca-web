use xitca_io::bytes::{Bytes, BytesMut};

#[derive(Debug)]
pub(super) struct Codec;

impl Codec {
    pub(super) fn encode(&self, msg: BytesMut) -> [Bytes; 2] {
        let msg = msg.freeze();
        let len = (msg.len() as u64).to_be_bytes();
        let prefix = Bytes::copy_from_slice(&len);
        [prefix, msg]
    }
}
