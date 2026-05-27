use std::error::Error;
use std::fs::File;
use std::io::{self, Read};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::str::Utf8Error;
use std::sync::Arc;

use futures::future::join_all;
use tokio::io::{BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};

mod parse;
use parse::parse_request;

mod respond;
use respond::{Response, write_response};

mod config;
use config::parse_config;

use crate::config::{Content, Route, Server};
use crate::parse::Request;

fn read_file(path: &str) -> Result<Vec<u8>, io::Error> {
    let mut result = Vec::new();
    File::open(path)?.read_to_end(&mut result)?;
    return Ok(result);
}

fn get_host(request: &Request) -> Result<&str, Utf8Error> {
    if !request.start_line.uri.host.is_empty() {
        Ok(&request.start_line.uri.host)
    } else {
        let host_port = str::from_utf8(&request.headers["Host"])?;
        if let Some(host_end) = host_port.find(':') {
            Ok(&host_port[..host_end])
        } else {
            Ok(host_port)
        }
    }
}

fn find_matching_route<'a>(server: &'a Server, path: &str) -> Option<&'a Route> {
    for route in &server.routes {
        if route.uri.starts_with(path) {
            return Some(route);
        }
    }
    None
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
        if let Some(route) = find_matching_route(server.as_ref(), &request.start_line.uri.path) {
            route
        } else {
            return Ok(());
        };

    let response = match &route.content {
        Content::NoContent => Response {
            status: route.status.unwrap_or(200),
            headers: route.headers.clone(),
            body: Vec::new(),
        },
        Content::FileAny(files) => {
            let mut body = Vec::new();
            for file in files {
                if file.exists() {
                    body = read_file("index.html")?;
                }
            }

            let mut headers = route.headers.clone();
            headers.insert(
                "Content-Length".to_owned(),
                itoa::Buffer::new().format(body.len()).as_bytes().to_vec(),
            );

            Response {
                status: route.status.unwrap_or(200),
                headers: headers,
                body: body,
            }
        }
        Content::Redirect(uri) => {
            let mut headers = route.headers.clone();
            headers.insert("Location".to_owned(), uri.clone().into_bytes());

            Response {
                status: route.status.unwrap_or(308),
                headers: headers,
                body: Vec::new(),
            }
        }
    };

    let mut writer = BufWriter::new(socket);
    write_response(&mut writer, &response).await?;

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
    })).await;

    Ok(())
}
