use std::fs::File;
use std::io::Read;
use std::{cmp::max_by_key, fs};
use std::{env, io};

use regex::Regex;
use thiserror::Error;

use crate::parse::Request;
use crate::{
    config::{Content, Route, Server, UriMatcher},
    respond::Response,
};

#[derive(Debug, Error)]
pub enum VarError {
    #[error("Unknow variable {0}")]
    UnknownVar(String),
}

fn to_pascal_case(str: &str) -> String {
    let mut result = String::new();
    let mut prev_letter = false;
    for c in str.chars() {
        if c.is_ascii_lowercase() && !prev_letter {
            result.push(c.to_ascii_uppercase());
        } else if c == '_' {
            result.push('-');
        } else {
            result.push(c);
        }

        prev_letter = c.is_ascii_alphabetic();
    }
    result
}

fn write_var(var: &str, output: &mut String, request: &Request) -> Result<(), VarError> {
    if let Some(header) = var.strip_prefix("http_") {
        let header = to_pascal_case(header);
        if let Some(value) = request.headers.get(&header) {
            output.push_str(value);
            return Ok(());
        }
    }

    if let Some(env) = var.strip_prefix("env_") {
        let env = env.to_ascii_uppercase();
        if let Ok(data) = env::var(env) {
            output.push_str(&data);
            return Ok(());
        }
    }

    match var {
        "uri" => output.push_str(&request.start_line.uri.full),
        "host" => {
            if request.start_line.uri.host.is_empty() {
                output.push_str(
                    &request
                        .headers
                        .get("Host")
                        .map(|s| s.as_str())
                        .unwrap_or(""),
                );
            } else {
                output.push_str(&request.start_line.uri.host)
            }
        }
        "method" => output.push_str(&request.start_line.uri.host),
        _ => return Err(VarError::UnknownVar(var.to_owned())),
    }
    Ok(())
}

fn replace_vars(str: &str, request: &Request) -> Result<String, VarError> {
    let mut result = String::new();

    let mut is_var = false;
    let mut last_var = String::new();

    for (_, c) in str.chars().enumerate() {
        if is_var {
            if c.is_ascii_alphanumeric() || c == '_' {
                last_var.push(c);
                continue;
            } else {
                write_var(&last_var, &mut result, request)?;

                is_var = false;
                last_var.clear();
            }
        }

        if c == '$' {
            is_var = true;
        } else {
            result.push(c);
        }
    }

    if is_var {
        write_var(&last_var, &mut result, request)?;
    }

    Ok(result)
}

pub fn find_matching_route<'a>(server: &'a Server, path: &str) -> Option<&'a Route> {
    let mut prioritised = None::<&Route>;
    let mut regex = None::<&Route>;
    let mut prefix = None::<&Route>;

    for route in &server.routes {
        match route.matcher {
            UriMatcher::Prefix => {
                if path.starts_with(&route.uri) {
                    prefix = max_by_key(prefix, Some(route), |route| {
                        route.map(|route| route.uri.len())
                    })
                }
            }
            UriMatcher::Regex => {
                if Regex::new(&route.uri).unwrap().find(path).is_some() && regex.is_none() {
                    regex = Some(route);
                }
            }
            UriMatcher::PrefixPrioritiesed => {
                if path.starts_with(&route.uri) {
                    prioritised = max_by_key(prioritised, Some(route), |route| {
                        route.map(|route| route.uri.len())
                    })
                }
            }
            UriMatcher::Exact => {
                if path == route.uri {
                    prioritised = max_by_key(prioritised, Some(route), |route| {
                        route.map(|route| route.uri.len())
                    })
                }
            }
        }
    }

    if prioritised.is_some() {
        return prioritised;
    } else if regex.is_some() {
        return regex;
    } else {
        return prefix;
    }
}

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
}

pub fn construct_response(route: &Route, request: &Request) -> Result<Response, ResponseError> {
    let mut headers = route
        .headers
        .iter()
        .map(|(k, v)| Ok((k.clone(), replace_vars(v, request)?)))
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
            body: replace_vars(data, request)
                .map_err(ResponseError::VarError)?
                .as_bytes()
                .to_owned(),
        },
        Some(Content::FileAny(files)) => {
            let mut body = Vec::new();
            for file in files {
                let file = replace_vars(&file, request).map_err(ResponseError::VarError)?;

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
            let uri = replace_vars(&uri, request).map_err(ResponseError::VarError)?;
            headers.insert("Location".to_owned(), uri.clone());

            Response {
                status: route.status.unwrap_or(308),
                headers: headers,
                body: Vec::new(),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::config::{Content, Route, Server, UriMatcher};
    use crate::process::find_matching_route;

    #[test]
    fn test_find_matching_route() {
        let server = Server {
            ip: None,
            port: Some(8080),
            domain_names: Vec::new(),
            is_default: true,
            routes: Vec::from([
                Route {
                    uri: "/exact".to_string(),
                    matcher: UriMatcher::Exact,
                    content: Some(Content::NoContent),
                    status: Some(201),
                    headers: HashMap::new(),
                },
                Route {
                    uri: "^/e.*t$".to_string(),
                    matcher: UriMatcher::Regex,
                    content: Some(Content::NoContent),
                    status: Some(202),
                    headers: HashMap::new(),
                },
                Route {
                    uri: "/exa".to_string(),
                    matcher: UriMatcher::PrefixPrioritiesed,
                    content: Some(Content::NoContent),
                    status: Some(203),
                    headers: HashMap::new(),
                },
                Route {
                    uri: "/ex".to_string(),
                    matcher: UriMatcher::Exact,
                    content: Some(Content::NoContent),
                    status: Some(204),
                    headers: HashMap::new(),
                },
            ]),
        };

        assert_eq!(
            find_matching_route(&server, "/exact")
                .unwrap()
                .status
                .unwrap(),
            201
        );
        assert_eq!(
            find_matching_route(&server, "/exct")
                .unwrap()
                .status
                .unwrap(),
            202
        );
        assert_eq!(
            find_matching_route(&server, "/exat")
                .unwrap()
                .status
                .unwrap(),
            203
        );
        assert_eq!(
            find_matching_route(&server, "/ex").unwrap().status.unwrap(),
            204
        );
    }
}
