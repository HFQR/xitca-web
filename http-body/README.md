## alternative of http-body crate adressing issue:

- `SizeHint::None` for expression no http body
- `futures::Stream` trait impl for body type avoid nesting adapter
- `Frame::Data` and `Frame::Trailers` are public for simply matching
