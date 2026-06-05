use std::fs;
use std::fs::File;
use std::io;
use std::io::Read;

use thiserror::Error;

use tokio::io::{BufReader, BufWriter};
use tokio::net::TcpStream;

use crate::config::{Content, Route};
use crate::http::{HttpVersion, ParseError, Request, Response, parse_response, write_request};
use crate::vars::{VarError, replace_dynamic_vars};

fn read_file(path: &str) -> Result<Vec<u8>, io::Error> {
    let mut result = Vec::new();
    File::open(path)?.read_to_end(&mut result)?;
    return Ok(result);
}

#[derive(Debug, Error)]
pub enum ResponseError {
    #[error("Io error: {0}")]
    IoError(io::Error),
    #[error("Variable error: {0}")]
    VarError(VarError),
    #[error("Parse response error: {0}")]
    ParseResponseError(ParseError),
}

pub async fn construct_response(
    route: &Route,
    request: &Request,
) -> Result<Response, ResponseError> {
    let mut headers = route
        .headers
        .iter()
        .map(|(k, v)| {
            Ok((
                replace_dynamic_vars(k, request)?,
                replace_dynamic_vars(v, request)?,
            ))
        })
        .collect::<Result<_, _>>()
        .map_err(ResponseError::VarError)?;

    Ok(match &route.content {
        None => match route.status {
            Some(200) | None => {
                let path = &request.start_line.uri.path;
                let path = path.strip_prefix('/').unwrap_or(path);

                if fs::exists(path).map_err(ResponseError::IoError)? {
                    Response {
                        status: 200,
                        headers: headers,
                        body: read_file(path).map_err(ResponseError::IoError)?,
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
                .map_err(ResponseError::VarError)?
                .as_bytes()
                .to_owned(),
        },
        Some(Content::FileAny(files)) => {
            let mut body = Vec::new();
            for file in files {
                let file = replace_dynamic_vars(&file, request).map_err(ResponseError::VarError)?;

                if fs::exists(&file).map_err(ResponseError::IoError)? {
                    body = read_file(&file).map_err(ResponseError::IoError)?;
                }
            }

            Response {
                status: route.status.unwrap_or(200),
                headers: headers,
                body: body,
            }
        }
        Some(Content::Redirect(uri)) => {
            let uri = replace_dynamic_vars(&uri, request).map_err(ResponseError::VarError)?;
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
            let uri = replace_dynamic_vars(&uri, request).map_err(ResponseError::VarError)?;

            let mut proxy_request = request.clone();
            proxy_request.start_line.version = HttpVersion(1, 1);

            if !proxy_request.start_line.uri.host.is_empty() {
                proxy_request.headers.insert(
                    "Host".to_string(),
                    proxy_request.start_line.uri.host.clone(),
                );
            }

            for (header_name, header_value) in proxy_headers {
                let header_name =
                    replace_dynamic_vars(header_name, &request).map_err(ResponseError::VarError)?;
                let header_value = replace_dynamic_vars(header_value, &request)
                    .map_err(ResponseError::VarError)?;

                proxy_request.headers.insert(header_name, header_value);
            }

            let mut stream = TcpStream::connect(uri)
                .await
                .map_err(ResponseError::IoError)?;

            write_request(&mut BufWriter::new(&mut stream), &proxy_request)
                .await
                .map_err(ResponseError::IoError)?;

            let mut response = parse_response(&mut BufReader::new(&mut stream))
                .await
                .map_err(ResponseError::ParseResponseError)?;

            for (header_name, header_value) in headers {
                response.headers.insert(header_name, header_value);
            }

            response
        }
    })
}
