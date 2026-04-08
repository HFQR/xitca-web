# unreleased 0.4.0
## Add
- add `client_request_extend` function for extending websocket headers/methods to an existing `Request` type.
- add `ResponseSender::{continuation, ping}` methods

## Change
- `client_request_from_uri` becomes infallible by receive `Uri` type without try conversion.
- `ProtocolError` covers more error condition and it's part in control flow has been updated
- `ResponseSender::text` method receives a type that can be converted to `Bytes` type. Enable possible zero copy String text message. Unless you already have a valid string type it would be more efficient to pass a u8 buffer to this API. utf-8 validation would be checked in method 

## Remove
- remove `ResponseSender::send` method to reduce possible misuse
- remove `ResponseSender::send_error` emthod

# 0.3.0
## Add
- add `RequestStream::inner_mut` method for accessing inner stream type.
- add `RequestStream::codec_mut` method for accessing `Codec` type.
- add `ResponseSender::close` method for sending close message.

## Change
- reduce `stream::RequestStream`'s generic type params. 
- `WsOutput` type does not bound to generic E type anymore.

# 0.2.0
## Add
- export `ResponseWeakSender` type when `stream` features is enabled.
- add `ResponseSender::send_error`.

## Change
- `ResponseStream` as `Stream` produce `std::io::Result<Bytes>` as `Stream::Item`. This enables `ResponseSender` sending arbitrary error type to signal the associated TCP io type to enter error handling path. See `ResponseSender::send_error` for example.
- `ResponseSender::send` would respect outgoing message queue limit(determined by `Codec::set_capacity`) and avoid aggressively encoding message.
