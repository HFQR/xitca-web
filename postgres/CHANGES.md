# unreleased 0.2.0
## Remove
- remove `query_raw`, `execute_raw`, `query_simple` and `execute_simple` methods from all types. Leave only `query` and `execute` as sole API. 
    ```rust
    // query and execute is able to handle raw string query.
    let stream = cli.query("SELECT 1; SELECT 1")?;
    let row_affected = cli.execute("SELECT 1; SELECT 1");
    ```
- remove `dev::AsParams` trait export. It's not needed for implementing `Query` trait anymore    

## Change
- query with parameter value arguments must be bind to it's `Statement` before received by `query` and `execute` APIs.
    ```rust
    // prepare a statement.
    let stmt = cli.prepare("SELECT * FROM users WHERE id = $1, age = $2", &[Type::INT4, Type::INT4]).await?;
    // bind statement to typed value.
    let bind = stmt.bind([9527, 42]);
    // query with the bind
    let stream = cli.query(bind)?;
    ```
- query without parameter value can be queried with `Statement` alone.
    ```rust
    // prepare a statement.
    let stmt = cli.prepare("SELECT * FROM users", &[]).await?;
    // statement have no value params and can be used for query.
    let stream = cli.query(&stmt)?;
    ```    
- `Query::_send_encode_query` method's return type is changed to `Result<(<S as Encode>::Output<'_>, Response), Error>`. Enabling further simplify of the surface level API at the cost of more internal complexity
- `Encode` and `IntoStream` traits implementation detail change

## Add
- add `statement::Statement::{bind, bind_dyn}` methods for binding value parameters to a prepared statement for query
- add `error::DriverIoErrorMulti` type for outputting read and write IO errors at the same time

## Fix
- remove `Clone` trait impl from `Statement`. this is a bug where `Statement` type is not meant to be duplicateable by library user
