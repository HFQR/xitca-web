use postgres_protocol::{
    authentication,
    message::{backend, frontend},
};
use xitca_io::{io::AsyncIo, net::TcpStream};

use crate::{client::Client, io::buffered_io::BufferedIo};

use super::{config::Config, error::Error, response::Response};

#[cold]
#[inline(never)]
pub(crate) async fn connect(cfg: &Config) -> Result<TcpStream, Error> {
    let host = cfg.get_hosts().first().unwrap();
    let port = cfg.get_ports().first().unwrap();

    match host {
        crate::config::Host::Tcp(host) => {
            let addrs = tokio::net::lookup_host((host.as_str(), *port)).await?;

            let mut err = None;

            for addr in addrs {
                match TcpStream::connect(addr).await {
                    Ok(stream) => {
                        let _ = stream.set_nodelay(true);
                        return Ok(stream);
                    }
                    Err(e) => err = Some(e),
                }
            }

            Err(err.unwrap().into())
        }
    }
}

#[cold]
#[inline(never)]
pub(crate) async fn authenticate<Io, const BATCH_LIMIT: usize>(
    client: &Client,
    io: &mut BufferedIo<Io, BATCH_LIMIT>,
    cfg: Config,
) -> Result<(), Error>
where
    Io: AsyncIo,
{
    let mut params = vec![("client_encoding", "UTF8")];
    if let Some(user) = &cfg.user {
        params.push(("user", &**user));
    }
    if let Some(dbname) = &cfg.dbname {
        params.push(("database", &**dbname));
    }
    if let Some(options) = &cfg.options {
        params.push(("options", &**options));
    }
    if let Some(application_name) = &cfg.application_name {
        params.push(("application_name", &**application_name));
    }

    let mut res = io
        .linear_request(client, |buf| {
            frontend::startup_message(params, buf).map_err(|_| Error::ToDo)
        })
        .await?;

    match res.recv().await? {
        backend::Message::AuthenticationOk => {}
        backend::Message::AuthenticationCleartextPassword => {
            let pass = cfg.get_password().unwrap();
            password(client, &mut res, io, pass).await?
        }
        backend::Message::AuthenticationMd5Password(body) => {
            let user = cfg.get_user().unwrap().as_bytes();
            let pass = cfg.get_password().unwrap();
            let pass = authentication::md5_hash(user, pass, body.salt());

            password(client, &mut res, io, pass).await?
        }
        _ => {}
    };

    // let (process_id, secret_key) = read_info(res).await?;
    //
    // println!("process_id: {}", process_id);
    // println!("secret_key: {}", secret_key);

    Ok(())
}

#[cold]
#[inline(never)]
async fn password<Io, P, const BATCH_LIMIT: usize>(
    client: &Client,
    res: &mut Response,
    io: &mut BufferedIo<Io, BATCH_LIMIT>,
    pass: P,
) -> Result<(), Error>
where
    Io: AsyncIo,
    P: AsRef<[u8]>,
{
    let _ = io
        .linear_request(client, |buf| {
            frontend::password_message(pass.as_ref(), buf).map_err(|_| Error::ToDo)
        })
        .await?;

    match res.recv().await? {
        backend::Message::AuthenticationOk => tracing::trace!("authenticated successful"),
        _ => panic!("password authentication failed"),
    }

    Ok(())
}

// #[cold]
// #[inline(never)]
// async fn read_info(mut res: Response) -> Result<(i32, i32), Error> {
//     let mut process_id = 0;
//     let mut secret_key = 0;
//
//     loop {
//         match res.recv().await? {
//             backend::Message::BackendKeyData(body) => {
//                 process_id = body.process_id();
//                 secret_key = body.secret_key();
//             }
//             backend::Message::ParameterStatus(_) => {
//                 // TODO: do not ignore parameter status.
//             }
//             _msg @ backend::Message::NoticeResponse(_) => {
//                 // TODO: do not ignore notice response status.
//             }
//             backend::Message::ReadyForQuery(_) => return Ok((process_id, secret_key)),
//             backend::Message::ErrorResponse(_) => return Err(Error::ToDo),
//             _ => return Err(Error::ToDo),
//         }
//     }
// }
