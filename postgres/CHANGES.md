# unreleased 0.2.0
## Add
- add `error::DriverIoErrorMulti` type for outputting read and write IO errors at the same time

## Change
- `Encode` and `IntoStream` traits implementation detail change
- `Query::_send_encode_query` method's return type is changed to `Result<(<S as Encode>::Output<'_>, Response), Error>`. Enabling further simplify of the surface level API at the cost of more internal complexity.

## Fix
- remove `Clone` trait impl from `Statement`. this is a bug where `Statement` type is not meant to be duplicateable by library user
