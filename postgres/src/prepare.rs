use std::sync::atomic::{AtomicUsize, Ordering};

use fallible_iterator::FallibleIterator;
use postgres_protocol::message::{backend, frontend};
use postgres_types::Type;
use tracing::debug;
use xitca_io::bytes::{Bytes, BytesMut};

use super::{client::Client, error::Error, statement::Statement};

impl Client {
    pub async fn prepare(&self, query: &str, types: &[Type]) -> Result<Statement, Error> {
        let buf = prepare_buf(&mut *self.buf.borrow_mut(), query, types)?;

        let mut res = self.send(buf).await?;

        match res.recv().await? {
            backend::Message::ParseComplete => {}
            _ => return Err(Error::ToDo),
        }

        let parameter_description = match res.recv().await? {
            backend::Message::ParameterDescription(body) => body,
            _ => return Err(Error::ToDo),
        };

        let row_description = match res.recv().await? {
            backend::Message::RowDescription(body) => Some(body),
            backend::Message::NoData => None,
            _ => return Err(Error::ToDo),
        };

        // let mut parameters = vec![];
        // let mut it = parameter_description.parameters();
        // while let Some(oid) = it.next().map_err(Error::parse)? {
        //     let type_ = get_type(client, oid).await?;
        //     parameters.push(type_);
        // }

        // let mut columns = vec![];
        // if let Some(row_description) = row_description {
        //     let mut it = row_description.fields();
        //     while let Some(field) = it.next().map_err(Error::parse)? {
        //         let type_ = get_type(client, field.type_oid()).await?;
        //         let column = Column::new(field.name().to_string(), type_);
        //         columns.push(column);
        //     }
        // }

        // Ok(Statement::new(client, name, parameters, columns))

        todo!()
    }
}

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

fn prepare_buf(buf: &mut BytesMut, query: &str, types: &[Type]) -> Result<Bytes, Error> {
    let name = &format!("s{}", NEXT_ID.fetch_add(1, Ordering::SeqCst));

    if types.is_empty() {
        debug!("preparing query {}: {}", name, query);
    } else {
        debug!("preparing query {} with types {:?}: {}", name, types, query);
    }

    frontend::parse(name, query, types.iter().map(Type::oid), buf)?;
    frontend::describe(b'S', name, buf)?;
    frontend::sync(buf);

    Ok(buf.split().freeze())
}
