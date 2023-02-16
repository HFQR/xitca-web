// use fallible_iterator::FallibleIterator;
use postgres_protocol::{
    authentication::{
        self,
        // sasl
    },
    message::{backend, frontend},
};
use xitca_io::net::TcpStream;

use super::{client::Client, config::Config, error::AuthenticationError, error::Error, response::Response};

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
pub(crate) async fn authenticate(cli: &Client, cfg: Config) -> Result<(), Error> {
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

    let msg = cli.with_buf_fallible(|buf| frontend::startup_message(params, buf).map(|_| buf.split()))?;

    let mut res = cli.send(msg)?;

    match res.recv().await? {
        backend::Message::AuthenticationOk => {}
        backend::Message::AuthenticationCleartextPassword => {
            let pass = cfg.get_password().ok_or(AuthenticationError::MissingPassWord)?;
            password(cli, &mut res, pass).await?
        }
        backend::Message::AuthenticationMd5Password(body) => {
            let pass = cfg.get_password().ok_or(AuthenticationError::MissingPassWord)?;
            let user = cfg.get_user().ok_or(AuthenticationError::MissingUserName)?.as_bytes();
            let pass = authentication::md5_hash(user, pass, body.salt());
            password(cli, &mut res, pass).await?
        }
        backend::Message::AuthenticationSasl(_body) => {
            todo!()
            // let pass = cfg.get_password().ok_or(AuthenticationError::MissingPassWord)?;
            //
            // let mut is_scram = false;
            // let mut is_scram_plus = false;
            // let mut mechanisms = body.mechanisms();
            // let channel_binding = None;
            //
            // while let Some(mechanism) = mechanisms.next()? {
            //     match mechanism {
            //         sasl::SCRAM_SHA_256 => is_scram = true,
            //         sasl::SCRAM_SHA_256_PLUS => is_scram_plus = true,
            //         _ => {}
            //     }
            // }
            //
            // let (channel_binding, mechanism) = match (channel_binding, is_scram_plus, is_scram) {
            //     // TODO: return "unsupported SASL mechanism" error.
            //     (_, false, false) => return Err(Error::ToDo),
            //     (Some(binding), true, _) => (binding, sasl::SCRAM_SHA_256_PLUS),
            //     (Some(_), false, true) => (sasl::ChannelBinding::unrequested(), sasl::SCRAM_SHA_256),
            //     (None, _, _) => (sasl::ChannelBinding::unsupported(), sasl::SCRAM_SHA_256),
            // };
            //
            // let mut scram = sasl::ScramSha256::new(pass, channel_binding);
            //
            // io.linear_request(|buf| frontend::sasl_initial_response(mechanism, scram.message(), buf))
            //     .await?;
            //
            // let body = match res.recv().await? {
            //     backend::Message::AuthenticationSaslContinue(body) => body,
            //     _ => return Err(Error::ToDo),
            // };
            //
            // scram.update(body.data())?;
            //
            // io.linear_request(|buf| frontend::sasl_response(scram.message(), buf))
            //     .await?;
            //
            // let body = match res.recv().await? {
            //     backend::Message::AuthenticationSaslFinal(body) => body,
            //     _ => return Err(Error::ToDo),
            // };
            //
            // scram.finish(body.data())?;
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
async fn password<P>(cli: &Client, res: &mut Response, pass: P) -> Result<(), Error>
where
    P: AsRef<[u8]>,
{
    let msg = cli.with_buf_fallible(|buf| frontend::password_message(pass.as_ref(), buf).map(|_| buf.split()))?;

    cli.send(msg)?;

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
