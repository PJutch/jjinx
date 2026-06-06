use std::error::Error;
use std::fs::File;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::str::Utf8Error;
use std::sync::Arc;

use futures::future::join_all;
use tokio::io::{BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};

mod http;
use http::{Request, parse_request, write_response};

mod config;
use config::{Server, parse_config};

mod process;
use process::construct_response;

mod route_matching;
use route_matching::find_matching_route;

mod vars;

mod uri;

fn get_host(request: &Request) -> Result<&str, Utf8Error> {
    if !request.start_line.uri.host().is_empty() {
        Ok(&request.start_line.uri.host())
    } else {
        let host_port = &request.headers["Host"];
        if let Some(host_end) = host_port.find(':') {
            Ok(&host_port[..host_end])
        } else {
            Ok(host_port)
        }
    }
}

async fn process_connection(
    socket: &mut TcpStream,
    server: Arc<Server>,
) -> Result<(), Box<dyn Error>> {
    let mut reader = BufReader::new(&mut *socket);
    let request = parse_request(&mut reader).await?;

    let host = get_host(&request)?;
    if !server.domain_names.iter().any(|name| name == host) && !server.is_default {
        return Ok(());
    }

    let route =
        if let Some(route) = find_matching_route(server.as_ref(), request.start_line.uri.path()) {
            route
        } else {
            return Ok(());
        };

    let mut writer = BufWriter::new(socket);
    write_response(&mut writer, &construct_response(route, &request).await?).await?;

    Ok(())
}

async fn run_server(server: Server) {
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
        let (mut socket, _addr) = match listener.accept().await {
            Ok(connection) => connection,
            Err(err) => {
                println!("{err}");
                continue;
            }
        };

        let server = server.clone();
        tokio::spawn(async move {
            if let Err(err) = process_connection(&mut socket, server).await {
                println!("{err}");
            }
        });
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = parse_config(&mut std::io::BufReader::new(File::open("jjinx.conf")?))?;

    join_all(config.into_iter().map(|server| {
        tokio::spawn(async move {
            run_server(server).await;
        })
    }))
    .await;

    Ok(())
}
