## alternative of http-body crate adressing issue:

- `SizeHint::None` for expression no http body
- `futures::Stream` trait impl for body type avoid nesting adapter
- crate hack for expressing `SizeHint::None` with `futures::Stream` trait
- `Frame::Data` and `Frame::Trailers` are public for simple matching
