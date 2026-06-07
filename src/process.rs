use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;

use crate::config::{Content, Server};
use crate::http::{self, ParseError, Request, Response, SendError, parse_request, write_response};
use crate::proxy::{ProxyError, Upstream, proxy_pass};
use crate::route_matching::{Match, find_matching_route};
use crate::uri::{self, parse_uri};
use crate::vars::{VarError, replace_dynamic_vars, replace_dynamic_vars_headers};

use std::net::IpAddr;

use tokio::io::{BufReader, BufWriter};
use tokio::net::TcpStream;

fn read_file(path: &str) -> Result<Vec<u8>, io::Error> {
    let mut result = Vec::new();
    File::open(path)?.read_to_end(&mut result)?;
    return Ok(result);
}

#[derive(Debug, Error)]
enum ServerError {
    #[error("Bad request: {0}")]
    RequestParseError(http::ParseError),
    #[error("Io error: {0}")]
    IoError(io::Error),
    #[error("Variable error: {0}")]
    VarError(VarError),
    #[error("Uri parse error: {0}")]
    UriParseError(uri::ParseError),
    #[error("Proxy error: {0}")]
    ProxyError(ProxyError),
    #[error("Error writing response: {0}")]
    ResponseError(SendError),
}

async fn construct_response<'a>(
    matching: Match<'a>,
    request: &Request,
    upstreams: Arc<HashMap<String, Upstream>>,
) -> Result<Response, ServerError> {
    let Match {
        route,
        matched_prefix_len,
    } = matching;

    let mut headers =
        replace_dynamic_vars_headers(&route.headers, request).map_err(ServerError::VarError)?;

    Ok(match &route.content {
        None => match route.status {
            Some(200) | None => {
                let path = &request.start_line.uri.path();
                let path = path.strip_prefix('/').unwrap_or(path);

                if fs::exists(path).map_err(ServerError::IoError)? {
                    Response {
                        status: 200,
                        headers: headers,
                        body: read_file(path).map_err(ServerError::IoError)?,
                    }
                } else if route.status.is_some() {
                    Response {
                        status: 200,
                        headers: headers,
                        body: Vec::new(),
                    }
                } else {
                    Response {
                        status: 404,
                        headers: headers,
                        body: Vec::new(),
                    }
                }
            }
            Some(status) => Response {
                status,
                headers: headers,
                body: Vec::new(),
            },
        },
        Some(Content::NoContent) => Response {
            status: route.status.unwrap_or(200),
            headers: headers,
            body: Vec::new(),
        },
        Some(Content::RawData(data)) => Response {
            status: route.status.unwrap_or(200),
            headers: headers,
            body: replace_dynamic_vars(data, request)
                .map_err(ServerError::VarError)?
                .as_bytes()
                .to_owned(),
        },
        Some(Content::FileAny(files)) => {
            let mut body = Vec::new();
            let mut found = false;

            for file in files {
                let file = replace_dynamic_vars(&file, request).map_err(ServerError::VarError)?;

                if fs::exists(&file).map_err(ServerError::IoError)? {
                    body = read_file(&file).map_err(ServerError::IoError)?;
                    found = true;
                }
            }

            Response {
                status: route.status.unwrap_or(if found { 200 } else { 404 }),
                headers: headers,
                body: body,
            }
        }
        Some(Content::Redirect(uri)) => {
            let uri = replace_dynamic_vars(&uri, request).map_err(ServerError::VarError)?;
            headers.insert("Location".to_owned(), uri.clone());

            Response {
                status: route.status.unwrap_or(308),
                headers: headers,
                body: Vec::new(),
            }
        }
        Some(Content::Proxy {
            uri,
            headers: proxy_headers,
        }) => {
            let uri = replace_dynamic_vars(&uri, request).map_err(ServerError::VarError)?;
            let proxy_headers = replace_dynamic_vars_headers(&proxy_headers, request)
                .map_err(ServerError::VarError)?;

            let mut response = proxy_pass(
                &parse_uri(&uri).map_err(ServerError::UriParseError)?,
                request,
                matched_prefix_len,
                proxy_headers,
                upstreams,
            )
            .await
            .map_err(ServerError::ProxyError)?;

            for (header_name, header_value) in headers {
                response.headers.insert(header_name, header_value);
            }

            response
        }
    })
}

fn get_host(request: &Request) -> &str {
    if !request.start_line.uri.host().is_empty() {
        &request.start_line.uri.host()
    } else {
        let host_port = &request.headers["Host"];
        if let Some(host_end) = host_port.find(':') {
            &host_port[..host_end]
        } else {
            host_port
        }
    }
}

async fn process_request(
    socket: &mut TcpStream,
    ip: IpAddr,
    server: Arc<Server>,
    upstreams: Arc<HashMap<String, Upstream>>,
) -> Result<(), ServerError> {
    let mut reader = BufReader::new(&mut *socket);
    let request = parse_request(
        &mut reader,
        ip,
        server.header_timeout.unwrap_or(Duration::from_secs(60)),
        server.body_timeout.unwrap_or(Duration::from_secs(60)),
    )
    .await
    .map_err(ServerError::RequestParseError)?;

    let host = get_host(&request);
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
    write_response(
        &mut writer,
        &construct_response(route, &request, upstreams).await?,
        server.send_timeout.unwrap_or(Duration::from_secs(60)),
    )
    .await
    .map_err(ServerError::ResponseError)?;

    Ok(())
}

pub async fn process_connection(
    socket: &mut TcpStream,
    ip: IpAddr,
    server: Arc<Server>,
    upstreams: Arc<HashMap<String, Upstream>>,
) {
    let send_timeout = server.send_timeout;

    match process_request(socket, ip, server, upstreams).await {
        Ok(()) => {}
        Err(err @ ServerError::RequestParseError(ParseError::Timeout)) => {
            println!("Timeout {err}");

            let mut writer = BufWriter::new(socket);
            if let Err(err) = write_response(
                &mut writer,
                &Response {
                    status: 408,
                    headers: HashMap::new(),
                    body: Vec::new(),
                },
                send_timeout.unwrap_or(Duration::from_secs(60)),
            )
            .await
            {
                println!("Response error: {err}")
            }
        }
        Err(ServerError::RequestParseError(err)) => {
            println!("Bad request {err}");

            let mut writer = BufWriter::new(socket);
            if let Err(err) = write_response(
                &mut writer,
                &Response {
                    status: 400,
                    headers: HashMap::new(),
                    body: Vec::new(),
                },
                send_timeout.unwrap_or(Duration::from_secs(60)),
            )
            .await
            {
                println!("Response error: {err}")
            }
        }
        Err(ServerError::ResponseError(err)) => {
            println!("Io error while responding: {err}")
        }
        Err(err) => {
            println!("{err}");

            let mut writer = BufWriter::new(socket);
            if let Err(err) = write_response(
                &mut writer,
                &Response {
                    status: 500,
                    headers: HashMap::new(),
                    body: Vec::new(),
                },
                send_timeout.unwrap_or(Duration::from_secs(60)),
            )
            .await
            {
                println!("Response error: {err}")
            }
        }
    }
}
