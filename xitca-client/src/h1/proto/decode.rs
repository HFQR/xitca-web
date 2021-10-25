use http::{Response, StatusCode, Version};
use httparse::{Header, Status, EMPTY_HEADER};

use super::{
    context::Context,
    error::{Parse, ProtoError},
};

const MAX_HEADERS: usize = 128;

impl Context {
    pub fn decode(&mut self) -> Result<Response<()>, ProtoError> {
        let mut headers = [EMPTY_HEADER; MAX_HEADERS];

        let mut parsed = httparse::Response::new(&mut headers);

        match parsed.parse(self.buf())? {
            Status::Complete(size) => {
                let mut res = Response::new(());

                *res.version_mut() = if parsed.version.unwrap() == 1 {
                    Version::HTTP_11
                } else {
                    Version::HTTP_10
                };

                *res.status_mut() = StatusCode::from_u16(parsed.code.unwrap()).map_err(|_| Parse::StatusCode)?;

                Ok(res)
            }
            Status::Partial => todo!(),
        }

        // let (len, ver, status, h_len) = {
        //     let mut res = httparse::Response::new(&mut parsed);
        //     match res.parse(src)? {
        //         httparse::Status::Complete(len) => {
        //             let version = if res.version.unwrap() == 1 {
        //                 Version::HTTP_11
        //             } else {
        //                 Version::HTTP_10
        //             };
        //             let status = StatusCode::from_u16(res.code.unwrap())
        //                 .map_err(|_| ParseError::Status)?;
        //             HeaderIndex::record(src, res.headers, &mut headers);

        //             (len, version, status, res.headers.len())
        //         }
        //         httparse::Status::Partial => {
        //             return if src.len() >= MAX_BUFFER_SIZE {
        //                 error!("MAX_BUFFER_SIZE unprocessed data reached, closing");
        //                 Err(ParseError::TooLarge)
        //             } else {
        //                 Ok(None)
        //             }
        //         }
        //     }
        // };

        // let mut msg = ResponseHead::new(status);
        // msg.version = ver;

        // // convert headers
        // let length = msg.set_headers(&src.split_to(len).freeze(), &headers[..h_len])?;

        // // message payload
        // let decoder = if let PayloadLength::Payload(pl) = length {
        //     pl
        // } else if status == StatusCode::SWITCHING_PROTOCOLS {
        //     // switching protocol or connect
        //     PayloadType::Stream(PayloadDecoder::eof())
        // } else {
        //     // for HTTP/1.0 read to eof and close connection
        //     if msg.version == Version::HTTP_10 {
        //         msg.set_connection_type(ConnectionType::Close);
        //         PayloadType::Payload(PayloadDecoder::eof())
        //     } else {
        //         PayloadType::None
        //     }
        // };

        // Ok(Some((msg, decoder)))
    }
}

#[derive(Clone, Copy)]
struct HeaderIndex {
    name: (usize, usize),
    value: (usize, usize),
}

impl HeaderIndex {
    const fn new() -> Self {
        Self {
            name: (0, 0),
            value: (0, 0),
        }
    }

    fn record(bytes: &[u8], headers: &[Header<'_>], indices: &mut [Self]) {
        let bytes_ptr = bytes.as_ptr() as usize;

        headers.iter().zip(indices.iter_mut()).for_each(|(header, indice)| {
            let name_start = header.name.as_ptr() as usize - bytes_ptr;
            let value_start = header.value.as_ptr() as usize - bytes_ptr;

            let name_end = name_start + header.name.len();
            let value_end = value_start + header.value.len();

            indice.name = (name_start, name_end);
            indice.value = (value_start, value_end);
        });
    }
}
