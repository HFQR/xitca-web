# unreleased 0.3.0
## Remove 
- remove `ExecuteMut` trait. It's role is replaced by `impl Execute<&mut C>`
- remove `error::AuthenticationError` type. It's error condition is covered by `error::ConfigError`

## Change
- change `pool::Pool`'s dead connection detection lifecycle.

# 0.2.1
## Fix
- relax lifetime bound on various query types 

# 0.2.0
## Remove
- remove `prepare`, `query`, `execute`, `query_raw`, `execute_raw`, `query_simple` and `execute_simple` methods from all types. Leave only `Execute` trait family as sole API  
    ```rust
    use xitca_postgres::{Client, Execute, RowSimpleStream, RowStream, Statement};
    // create a named statement and execute it. on success returns a prepared statement
    let stmt: StatementGuarded<'_, Client> = Statement::named("SELECT 1").execute(&client).await?;
    // query with the prepared statement. on success returns an async row stream.
    let stream: RowStream<'_> = stmt.query(&client).await?;
    // query with raw string sql.
    let stream: RowSimpleStream<'_> = "SELECT 1; SELECT 1".query(&client).await?;
    // execute raw string sql.
    let row_affected: u64 = "SELECT 1; SELECT 1".execute(&client).await?;
    // execute sql file.
    let row_affected: u64 = std::path::Path::new("./foo.sql").execute(&client).await?;
    ```
- remove `Client::pipeline` and `Pool::pipeline`. `pipeline::Pipeline` type can be execute with `Execute::query` method
    
  remove `pipeline::Pipeline::query` and `pipeline::Pipeline` can be queried with `ExecuteMut::query_mut` method
    ```rust
    use xitca_postgres::Execute;
    // prepare statement and create a pipeline
    let stmt = Statement::named("SELECT 1", &[]).execute(&client).await?;
    let mut pipe = Pipeline::new();
    
    // use ExecuteMut trait to add query to pipeline
    use xitca_postgres::ExecuteMut;
    stmt.query_mut(&mut pipe)?;
    stmt.query_mut(&mut pipe)?;

    // use Execute trait to start pipeline query
    let pipe_stream = pipe.query(&client)?;
    ```
- remove `dev::AsParams` trait export. It's not needed for implementing `Query` trait anymore    

## Change
- query with parameter value arguments must be bind to it's `Statement` before calling `Execute` methods.
    ```rust
    use xitca_postgres::Execute;
    // prepare a statement.
    let stmt = Statement::named("SELECT * FROM users WHERE id = $1 AND age = $2", &[Type::INT4, Type::INT4]).execute(&client).await?;
    // bind statement to typed value and start query
    let stream = stmt.bind([9527, 42]).query(&client).await?;
    ```
- query without parameter value can be queried with `Statement` alone.
    ```rust
    use xitca_postgres::Execute;
    // prepare a statement.
    let stmt = Statement::named("SELECT * FROM users", &[]).execute(&client).await?;
    // statement have no value params and can be used for query.
    let stream = stmt.query(&client).await?;
    ```
- `AsyncLendingIterator` is no longer exported from crate's root path. use `iter::AsyncLendingIterator` instead
- `query::RowStreamOwned` and `row::RowOwned` are no longer behind `compat` crate feature anymore
- `statement::Statement::unnamed` must bind to value parameters with `bind` or `bind_dyn` before calling `Execute` methods.
    ```rust
    let stmt = Statement::unnamed("SELECT * FROM users WHERE id = $1", &[Type::INT4]);
    let row_stream = stmt.bind([9527]).query(&client).await?;
    ```
- `Query::_send_encode_query` method's return type is changed to `Result<(<S as Encode>::Output, Response), Error>`. Enabling further simplify of the surface level API at the cost of more internal complexity
- `Encode` trait implementation detail change
- `IntoStream` trait is renamed to `IntoResponse` with implementation detail change

## Add
- add `Execute`, `ExecuteMut`, `ExecuteBlocking` traits for extending query customization
- add `Prepare::_get_type_blocking`
- add `iter::AsyncLendingIteratorExt` for extending async iterator APIs
- add `statement::Statement::{bind, bind_dyn}` methods for binding value parameters to a prepared statement for query
- add `query::RowSimpleStreamOwned`
- add `error::DriverIoErrorMulti` type for outputting read and write IO errors at the same time

## Fix
- remove `Clone` trait impl from `Statement`. this is a bug where `Statement` type is not meant to be duplicateable by library user
