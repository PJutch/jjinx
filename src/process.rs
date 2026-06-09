use std::collections::HashMap;
use std::fs::{self, File};
use std::io;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;
use tokio_rustls::TlsConnector;

use crate::config::{Content, Server, ServerGroup};
use crate::http::{ParseError, Request, Response, parse_request, write_response};
use crate::proxy::{ProxyError, Upstream, proxy_pass};
use crate::route_matching::{Match, find_matching_route};
use crate::uri::{self, parse_uri};
use crate::vars::{VarError, replace_dynamic_vars, replace_dynamic_vars_headers};

use std::net::IpAddr;

use tokio::io::{AsyncRead, AsyncWrite, BufReader, BufWriter};

#[derive(Clone, Copy)]
enum ContentType {
    Unknown,
    Html,
    Css,
    Js,
    Json,
    Xml,
}

impl ContentType {
    fn by_path(path: &str) -> ContentType {
        if path.ends_with(".html") || path.ends_with(".htm") {
            ContentType::Html
        } else if path.ends_with(".css") {
            ContentType::Css
        } else if path.ends_with(".js") {
            ContentType::Js
        } else if path.ends_with(".json") {
            ContentType::Json
        } else if path.ends_with(".xml") {
            ContentType::Xml
        } else {
            ContentType::Unknown
        }
    }

    fn to_mime(self) -> Option<&'static str> {
        Some(match self {
            Self::Html => "text/html",
            Self::Css => "text/css",
            Self::Js => "application/javascript",
            Self::Json => "application/json",
            Self::Xml => "application/xml",
            Self::Unknown => return None,
        })
    }
}

fn read_file(path: &str) -> Result<Option<(Vec<u8>, ContentType)>, io::Error> {
    if !fs::exists(path)? {
        return Ok(None);
    }

    if fs::metadata(path)?.is_dir() {
        let mut path = path.to_owned();
        if !path.ends_with('/') {
            path.push('/');
        }
        path.push_str("index.html");

        if !fs::exists(&path)? {
            return Ok(None);
        }

        let mut result = Vec::new();
        File::open(path)?.read_to_end(&mut result)?;
        return Ok(Some((result, ContentType::Html)));
    } else {
        let mut result = Vec::new();
        File::open(path)?.read_to_end(&mut result)?;
        return Ok(Some((result, ContentType::by_path(path))));
    }
}

#[derive(Debug, Error)]
enum ServerError {
    #[error("Io error: {0}")]
    IoError(io::Error),
    #[error("Variable error: {0}")]
    VarError(VarError),
    #[error("Uri parse error: {0}")]
    UriParseError(uri::ParseError),
    #[error("Proxy error: {0}")]
    ProxyError(ProxyError),
    #[error("Not found")]
    NotFound,
}

async fn construct_response<'a>(
    matching: Match<'a>,
    request: &Request,
    upstreams: Arc<HashMap<String, Upstream>>,
    connector: TlsConnector,
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

                if let Some((data, content_type)) = read_file(path).map_err(ServerError::IoError)? {
                    if let Some(content_type) = content_type.to_mime() {
                        headers.insert("Content-Type".to_owned(), content_type.to_owned());
                    }

                    Response {
                        status: 200,
                        headers: headers,
                        body: data,
                    }
                } else if route.status.is_some() {
                    Response {
                        status: 200,
                        headers: headers,
                        body: Vec::new(),
                    }
                } else {
                    return Err(ServerError::NotFound);
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
            let mut body = None;

            for file in files {
                let mut file =
                    replace_dynamic_vars(&file, request).map_err(ServerError::VarError)?;
                file.push_str(&request.start_line.uri.path()[matching.matched_prefix_len..]);

                if let Some((data, content_type)) =
                    read_file(&file).map_err(ServerError::IoError)?
                {
                    body = Some((data, content_type));
                }
            }

            if let Some((data, content_type)) = body {
                if let Some(content_type) = content_type.to_mime() {
                    headers.insert("Content-Type".to_owned(), content_type.to_owned());
                }

                Response {
                    status: route.status.unwrap_or(200),
                    headers: headers,
                    body: data,
                }
            } else {
                return Err(ServerError::NotFound);
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
                connector,
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

async fn construct_error_page(
    status: i16,
    server: &Server,
    request: &Request,
    upstreams: Arc<HashMap<String, Upstream>>,
    connector: TlsConnector,
) -> Option<Response> {
    let error_page = server.error_pages.get(&status)?;
    let route = find_matching_route(server, error_page)?;

    let mut response = construct_response(route, &request, upstreams, connector)
        .await
        .ok()?;

    if response.status == 200 {
        response.status = status;
    }

    Some(response)
}

async fn write_error<Writer: AsyncWrite + Unpin>(
    socket: &mut Writer,
    status: i16,
    server: &Server,
    request: &Request,
    upstreams: Arc<HashMap<String, Upstream>>,
    connector: TlsConnector,
) {
    let mut writer = BufWriter::new(socket);
    if let Err(err) = write_response(
        &mut writer,
        &construct_error_page(status, server, request, upstreams, connector)
            .await
            .unwrap_or_else(|| Response {
                status: status,
                headers: HashMap::new(),
                body: Vec::new(),
            }),
        server.send_timeout.unwrap_or(Duration::from_secs(60)),
    )
    .await
    {
        println!("Response error: {err}")
    }
}

pub async fn process_connection<Stream: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut Stream,
    ip: IpAddr,
    servers: Arc<ServerGroup>,
    upstreams: Arc<HashMap<String, Upstream>>,
    connector: TlsConnector,
) {
    let default_server = &servers.servers[servers.default];

    let mut reader = BufReader::new(&mut *socket);
    let request = match parse_request(
        &mut reader,
        ip,
        servers.servers[servers.default]
            .header_timeout
            .unwrap_or(Duration::from_secs(60)),
        servers.servers[servers.default]
            .body_timeout
            .unwrap_or(Duration::from_secs(60)),
    )
    .await
    {
        Ok(request) => request,
        Err(ParseError::Timeout) => {
            println!("Parse request: Timeout");
            write_error(
                socket,
                408,
                default_server,
                &Request::empty(ip),
                upstreams,
                connector,
            )
            .await;
            return;
        }
        Err(ParseError::UnknownTransferEncoding(encoding)) => {
            println!("Unknown transfer encoding: {encoding}");
            write_error(
                socket,
                501,
                default_server,
                &Request::empty(ip),
                upstreams,
                connector,
            )
            .await;
            return;
        }
        Err(err) => {
            println!("Bad request: {err}");
            write_error(
                socket,
                400,
                default_server,
                &Request::empty(ip),
                upstreams,
                connector,
            )
            .await;
            return;
        }
    };

    let host = get_host(&request);
    let server = servers
        .servers
        .iter()
        .find(|server| server.domain_names.iter().any(|name| name == host))
        .unwrap_or(&servers.servers[servers.default]);

    let route = if let Some(route) = find_matching_route(server, request.start_line.uri.path()) {
        route
    } else {
        println!("No location match {}", request.start_line.uri.full);
        write_error(socket, 404, server, &request, upstreams, connector).await;
        return;
    };

    let response =
        match construct_response(route, &request, upstreams.clone(), connector.clone()).await {
            Ok(response) => response,
            Err(ServerError::NotFound) => {
                println!("File not found");
                write_error(socket, 404, server, &request, upstreams, connector).await;
                return;
            }
            Err(err) => {
                println!("Internal server error: {err}");
                write_error(socket, 500, server, &request, upstreams, connector).await;
                return;
            }
        };

    let mut writer = BufWriter::new(socket);
    match write_response(
        &mut writer,
        &response,
        server.send_timeout.unwrap_or(Duration::from_secs(60)),
    )
    .await
    {
        Ok(()) => {}
        Err(err) => {
            println!("Io error while responding: {err}")
        }
    }
}
