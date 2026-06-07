use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io;
use std::io::Read;
use std::sync::Arc;

use thiserror::Error;

use crate::config::Content;
use crate::http::{Request, Response};
use crate::proxy::{ProxyError, Upstream, proxy_pass};
use crate::route_matching::Match;
use crate::uri::{self, parse_uri};
use crate::vars::{VarError, replace_dynamic_vars, replace_dynamic_vars_headers};

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
    #[error("Uri parse error: {0}")]
    UriParseError(uri::ParseError),
    #[error("Proxy error: {0}")]
    ProxyError(ProxyError),
}

pub async fn construct_response<'a>(
    matching: Match<'a>,
    request: &Request,
    upstreams: Arc<HashMap<String, Upstream>>,
) -> Result<Response, ResponseError> {
    let Match {
        route,
        matched_prefix_len,
    } = matching;

    let mut headers =
        replace_dynamic_vars_headers(&route.headers, request).map_err(ResponseError::VarError)?;

    Ok(match &route.content {
        None => match route.status {
            Some(200) | None => {
                let path = &request.start_line.uri.path();
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
            let proxy_headers = replace_dynamic_vars_headers(&proxy_headers, request)
                .map_err(ResponseError::VarError)?;

            let mut response = proxy_pass(
                parse_uri(&uri).map_err(ResponseError::UriParseError)?,
                request,
                matched_prefix_len,
                proxy_headers,
                upstreams,
            )
            .await
            .map_err(ResponseError::ProxyError)?;

            for (header_name, header_value) in headers {
                response.headers.insert(header_name, header_value);
            }

            response
        }
    })
}
