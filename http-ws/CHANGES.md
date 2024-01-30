# unreleased
## Add
- add `RequestStream::inner_mut` method for accessing inner stream type.
- add `RequestStream::codec_mut` method for accessing `Codec` type.

# 0.2.0
## Add
- export `ResponseWeakSender` type when `stream` features is enabled.
- add `ResponseSender::send_error`.

## Change
- `ResponseStream` as `Stream` produce `std::io::Result<Bytes>` as `Stream::Item`. This enables `ResponseSender` sending arbitrary error type to signal the associated TCP io type to enter error handling path. See `ResponseSender::send_error` for example.
- `ResponseSender::send` would respect outgoing message queue limit(determined by `Codec::set_capacity`) and avoid aggressively encoding message.
