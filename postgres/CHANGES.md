# unreleased 0.2.0
## Remove
- remove `query`, `execute`, `query_raw`, `execute_raw`, `query_simple` and `execute_simple` methods from all types. Leave only `Execute` trait as sole query API  
    ```rust
    use xitca_postgres::Execute;
    let stream = "SELECT 1; SELECT 1".query(&client)?;
    let row_affected = "SELECT 1; SELECT 1".execute(&client).await?;
    ```
- remove `dev::AsParams` trait export. It's not needed for implementing `Query` trait anymore    

## Change
- query with parameter value arguments must be bind to it's `Statement` before calling `Execute` methods.
    ```rust
    use xitca_postgres::Execute;
    // prepare a statement.
    let stmt = client.prepare("SELECT * FROM users WHERE id = $1 AND age = $2", &[Type::INT4, Type::INT4]).await?;
    // bind statement to typed value and start query
    let stream = stmt.bind([9527, 42]).query(&client)?;
    ```
- query without parameter value can be queried with `Statement` alone.
    ```rust
    use xitca_postgres::Execute;
    // prepare a statement.
    let stmt = client.prepare("SELECT * FROM users", &[]).await?;
    // statement have no value params and can be used for query.
    let stream = stmt.query(&client)?;
    ```
- `AsyncLendingIterator` is no longer exported from crate's root path. use `iter::AsyncLendingIterator` instead
- `query::RowStreamOwned` and `row::RowOwned` are no longer behind `compat` crate feature anymore
- `statement::Statement::unnamed` must bind to value parameters with `bind` or `bind_dyn` before calling `Execute` methods.
    ```rust
    let stmt = Statement::unnamed("SELECT * FROM users WHERE id = $1", &[Type::INT4]);
    let row_stream = stmt.bind([9527]).query(&client);
    ```
- `Query::_send_encode_query` method's return type is changed to `Result<(<S as Encode>::Output<'_>, Response), Error>`. Enabling further simplify of the surface level API at the cost of more internal complexity
- `Encode` and `IntoStream` traits implementation detail change

## Add
- add `Execute` trait for extending query customization
- add `Client::prepare_blocking`
- add `Prepare::{_prepare_blocking, _get_type_blocking}`
- add `iter::AsyncLendingIteratorExt` for extending async iterator APIs
- add `statement::Statement::{bind, bind_dyn}` methods for binding value parameters to a prepared statement for query
- add `query::RowSimpleStreamOwned`
- add `error::DriverIoErrorMulti` type for outputting read and write IO errors at the same time

## Fix
- remove `Clone` trait impl from `Statement`. this is a bug where `Statement` type is not meant to be duplicateable by library user
