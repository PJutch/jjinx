use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;

use futures::future::join_all;
use tokio::net::TcpListener;

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

async fn run_server(server: Server, upstreams: Arc<HashMap<String, Upstream>>) {
    let ip = server.ip.unwrap_or(Ipv4Addr::from_bits(0));
    let port = server.port.unwrap_or(8080);

    let listener = match TcpListener::bind(SocketAddrV4::new(ip, port)).await {
        Ok(listener) => listener,
        Err(err) => {
            println!("{err}");
            return;
        }
    };

    let server = Arc::new(server);
    loop {
        let (mut socket, addr) = match listener.accept().await {
            Ok(connection) => connection,
            Err(err) => {
                println!("{err}");
                continue;
            }
        };

        let server = server.clone();
        let upstreams = upstreams.clone();
        tokio::spawn(async move {
            process_connection(&mut socket, addr.ip(), server, upstreams).await;
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
            run_server(server, upstreams).await;
        })
    }))
    .await;

    Ok(())
}
