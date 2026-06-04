use std::fs::File;
use std::io;
use std::io::Read;
use std::{cmp::max_by_key, fs};

use regex::Regex;

use crate::{
    config::{Content, Route, Server, UriMatcher},
    respond::Response,
};

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

pub fn construct_response(route: &Route, request_path: &str) -> Result<Response, io::Error> {
    Ok(match &route.content {
        None => match route.status {
            Some(200) | None => {
                let path = request_path.strip_prefix('/').unwrap_or(request_path);
                if fs::exists(path)? {
                    Response {
                        status: 200,
                        headers: route.headers.clone(),
                        body: read_file(path)?,
                    }
                } else if route.status.is_some() {
                    Response {
                        status: 200,
                        headers: route.headers.clone(),
                        body: Vec::new(),
                    }
                } else {
                    Response {
                        status: 404,
                        headers: route.headers.clone(),
                        body: Vec::new(),
                    }
                }
            }
            Some(status) => Response {
                status,
                headers: route.headers.clone(),
                body: Vec::new(),
            },
        },
        Some(Content::NoContent) => Response {
            status: route.status.unwrap_or(200),
            headers: route.headers.clone(),
            body: Vec::new(),
        },
        Some(Content::FileAny(files)) => {
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
        Some(Content::Redirect(uri)) => {
            let mut headers = route.headers.clone();
            headers.insert("Location".to_owned(), uri.clone().into_bytes());

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
