use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::io::BufReader;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;

use futures::future::join_all;
use rustls::ServerConfig;
use rustls::pki_types::PrivateKeyDer;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

mod http;

mod config;
use config::{Server, parse_config};

mod uri;

mod process;
use process::process_connection;

mod route_matching;

use crate::proxy::{Upstream, make_upstreams};

mod proxy;

mod vars;

#[derive(Debug, Error)]
#[error("No private keys in the file")]
struct NoPrivateKey;

async fn run_server(
    server: Server,
    upstreams: Arc<HashMap<String, Upstream>>,
) -> Result<(), Box<dyn Error>> {
    let ip = server.ip.unwrap_or(Ipv4Addr::from_bits(0));
    let port = server.port.unwrap_or(8080);

    let acceptor = if let Some(cert_path) = &server.cert_path {
        let mut cert_reader = BufReader::new(File::open(cert_path)?);
        let certs = rustls_pemfile::certs(&mut cert_reader).collect::<Result<_, _>>()?;

        let mut keys_reader = BufReader::new(File::open(server.keys_path.as_ref().unwrap())?);
        let key = rustls_pemfile::private_key(&mut keys_reader)?.ok_or(NoPrivateKey)?;

        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)?;

        Some(TlsAcceptor::from(Arc::new(config)))
    } else {
        None
    };

    let listener = TcpListener::bind(SocketAddrV4::new(ip, port)).await?;

    let server = Arc::new(server);
    loop {
        let (mut socket, addr) = listener.accept().await?;

        let server = server.clone();
        let upstreams = upstreams.clone();
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            if let Some(acceptor) = acceptor {
                match acceptor.accept(socket).await {
                    Ok(mut stream) => {
                        process_connection(&mut stream, addr.ip(), server, upstreams).await;
                    }
                    Err(err) => {
                        println!("Tls accept error: {err}");
                    }
                }
            } else {
                process_connection(&mut socket, addr.ip(), server, upstreams).await;
            };
        });
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = parse_config(&mut std::io::BufReader::new(File::open("jjinx.conf")?))?;

    let upstreams = Arc::new(make_upstreams(config.upstreams));

    join_all(config.servers.into_iter().map(|server| {
        let upstreams = upstreams.clone();
        tokio::spawn(async move {
            if let Err(err) = run_server(server, upstreams).await {
                println!("Error starting a server: {err}");
            }
        })
    }))
    .await;

    Ok(())
}
