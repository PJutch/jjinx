use std::cmp::max_by_key;

use regex::Regex;

use crate::config::{Route, Server, UriMatcher};

pub struct Match<'a> {
    pub route: &'a Route,
    pub matched_prefix_len: usize,
}

pub fn find_matching_route<'a>(server: &'a Server, path: &str) -> Option<Match<'a>> {
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

    if let Some(prioritised) = prioritised {
        Some(Match {
            route: prioritised,
            matched_prefix_len: prioritised.uri.len(),
        })
    } else if let Some(regex) = regex {
        Some(Match {
            route: regex,
            matched_prefix_len: 0,
        })
    } else if let Some(prefix) = prefix {
        Some(Match {
            route: prefix,
            matched_prefix_len: prefix.uri.len(),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use super::find_matching_route;
    use crate::config::{Content, Route, Server, UriMatcher};

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
            header_timeout: None,
            body_timeout: None,
            send_timeout: None,
        };

        assert_eq!(
            find_matching_route(&server, "/exact")
                .unwrap()
                .route
                .status
                .unwrap(),
            201
        );
        assert_eq!(
            find_matching_route(&server, "/exct")
                .unwrap()
                .route
                .status
                .unwrap(),
            202
        );
        assert_eq!(
            find_matching_route(&server, "/exat")
                .unwrap()
                .route
                .status
                .unwrap(),
            203
        );
        assert_eq!(
            find_matching_route(&server, "/ex")
                .unwrap()
                .route
                .status
                .unwrap(),
            204
        );
    }
}
