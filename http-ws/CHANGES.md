# unreleased version 0.2.0

## Add
- export `ResponseWeakSender` type when `stream` features is enabled.
- add `ResponseSender::send_error`.

## Change
- `ResponseStream` as `Stream` produce `std::io::Result<Bytes>` as `Stream::Item`. This enables `ResponseSender` sending arbitrary error type to signal the associated TCP io type to enter error handling path. See [ResponseSender::send_error] for example.
