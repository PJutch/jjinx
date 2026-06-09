use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use futures::future::join_all;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

mod http;

mod config;
use config::parse_config;

mod uri;

mod process;
use process::process_connection;

mod route_matching;

use crate::config::ServerGroup;
use crate::proxy::{Upstream, make_upstreams};

mod proxy;

mod vars;

mod tls;
use tls::{make_acceptor, make_connector};

async fn handle_ip_port(
    ip: IpAddr,
    port: u16,
    servers: ServerGroup,
    upstreams: Arc<HashMap<String, Upstream>>,
) -> Result<(), Box<dyn Error>> {
    let acceptor = make_acceptor(&servers)?;
    let connector = make_connector()?;

    let listener = TcpListener::bind(SocketAddr::new(ip, port)).await?;

    let servers = Arc::new(servers);

    loop {
        let (mut socket, addr) = listener.accept().await?;

        let servers = servers.clone();
        let upstreams = upstreams.clone();
        let acceptor = acceptor.clone();
        let connector = connector.clone();

        tokio::spawn(async move {
            if let Some(acceptor) = acceptor {
                match acceptor.accept(&mut socket).await {
                    Ok(mut stream) => {
                        process_connection(&mut stream, addr.ip(), servers, upstreams, connector)
                            .await;

                        match stream.shutdown().await {
                            Ok(()) => {}
                            Err(err) => {
                                println!("Shutdown error {err}");
                            }
                        }
                    }
                    Err(err) => {
                        println!("Tls accept error: {err}");
                    }
                }
            } else {
                process_connection(&mut socket, addr.ip(), servers, upstreams, connector).await;

                match socket.shutdown().await {
                    Ok(()) => {}
                    Err(err) => {
                        println!("Shutdown error {err}");
                    }
                }
            };
        });
    }
}

/// A webserver/reverse-proxy
#[derive(Debug, Parser)]
struct Args {
    /// Config path
    #[arg(short, long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    let config = parse_config(&mut std::io::BufReader::new(File::open(
        args.config.unwrap_or_else(|| "jjinx.conf".into()),
    )?))?;

    let upstreams = Arc::new(make_upstreams(config.upstreams));

    join_all(config.servers.into_iter().map(|((ip, port), servers)| {
        let upstreams = upstreams.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_ip_port(ip, port, servers, upstreams).await {
                println!("Error starting a server: {err}");
            }
        })
    }))
    .await;

    Ok(())
}
