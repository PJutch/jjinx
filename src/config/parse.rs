use crate::config::{Balancing, Config, Upstream};
use crate::vars::replace_static_vars;
use std::path::PathBuf;
use std::time::Duration;
use std::{collections::HashMap, io::BufRead};

use super::tokenizer::{consume_fixed, next_token, read_tokens_until_newline};
use super::{Content, ParseError, Route, Server, UriMatcher};

fn parse_set<Reader: BufRead>(
    reader: &mut Reader,
    vars: &[&HashMap<String, String>],
) -> Result<(String, String), ParseError> {
    let var = replace_static_vars(&next_token(reader)?, vars).unwrap();
    let value = replace_static_vars(&next_token(reader)?, vars).unwrap();
    consume_fixed(reader, "\n")?;

    Ok((var, value))
}

fn parse_route<Reader: BufRead>(
    reader: &mut Reader,
    global_vars: &HashMap<String, String>,
    server_vars: &HashMap<String, String>,
) -> Result<Route, ParseError> {
    let mut route = Route::default();

    let token1 = replace_static_vars(&next_token(reader)?, &[server_vars, global_vars]).unwrap();
    let token2 = replace_static_vars(&next_token(reader)?, &[server_vars, global_vars]).unwrap();

    if token2 == "{" {
        route.uri = token1;
    } else {
        route.matcher = match token1.as_str() {
            "=" => UriMatcher::Exact,
            "~" | "~*" => UriMatcher::Regex,
            "^~" => UriMatcher::PrefixPrioritiesed,
            _ => return Err(ParseError::UnknownMatcher(token1)),
        };

        if token1 == "~*" {
            route.uri += "(?i)";
            route.uri += &token2;
        } else {
            route.uri = token2;
        }

        consume_fixed(reader, "{")?;
    }

    let mut vars = HashMap::new();

    loop {
        let token = next_token(reader)?;
        match token.as_str() {
            "status" => {
                let status =
                    replace_static_vars(&next_token(reader)?, &[&vars, server_vars, global_vars])
                        .unwrap()
                        .parse()
                        .map_err(ParseError::ParseIntError)?;

                consume_fixed(reader, "\n")?;

                if route.status.is_none() {
                    route.status = Some(status)
                } else {
                    return Err(ParseError::DuplicateField(token));
                }
            }
            "nocontent" => {
                if route.content.is_none() {
                    route.content = Some(Content::NoContent);
                } else {
                    return Err(ParseError::DuplicateContent(route.uri));
                }
            }
            "body" => {
                let body =
                    replace_static_vars(&next_token(reader)?, &[&vars, server_vars, global_vars])
                        .unwrap();
                consume_fixed(reader, "\n")?;

                if route.content.is_none() {
                    route.content = Some(Content::RawData(body));
                } else {
                    return Err(ParseError::DuplicateContent(route.uri));
                }
            }
            "file" => {
                let mut paths = read_tokens_until_newline(reader)?
                    .iter()
                    .map(|path| {
                        replace_static_vars(path, &[&vars, server_vars, global_vars]).unwrap()
                    })
                    .collect();

                match route.content {
                    None => route.content = Some(Content::FileAny(paths)),
                    Some(Content::FileAny(mut files)) => {
                        files.append(&mut paths);
                        route.content = Some(Content::FileAny(files));
                    }
                    Some(_) => return Err(ParseError::DuplicateContent(route.uri)),
                }
            }
            "redirect" => {
                let uri =
                    replace_static_vars(&next_token(reader)?, &[&vars, server_vars, global_vars])
                        .unwrap();
                consume_fixed(reader, "\n")?;

                if route.content.is_none() {
                    route.content = Some(Content::Redirect(uri));
                } else {
                    return Err(ParseError::DuplicateContent(route.uri));
                }
            }
            "proxy" => {
                let uri =
                    replace_static_vars(&next_token(reader)?, &[&vars, server_vars, global_vars])
                        .unwrap();
                consume_fixed(reader, "\n")?;

                match &mut route.content {
                    None => {
                        route.content = Some(Content::Proxy {
                            uri,
                            headers: HashMap::new(),
                        });
                    }
                    Some(Content::Proxy {
                        uri: current_uri, ..
                    }) if current_uri.is_empty() => {
                        *current_uri = uri;
                    }
                    Some(_) => return Err(ParseError::DuplicateContent(route.uri)),
                }
            }
            "proxy_header" => {
                let header_name =
                    replace_static_vars(&next_token(reader)?, &[&vars, server_vars, global_vars])
                        .unwrap();
                let header_value =
                    replace_static_vars(&next_token(reader)?, &[&vars, server_vars, global_vars])
                        .unwrap();
                consume_fixed(reader, "\n")?;

                match &mut route.content {
                    None => {
                        route.content = Some(Content::Proxy {
                            uri: "".to_owned(),
                            headers: HashMap::from([(header_name, header_value)]),
                        })
                    }
                    Some(Content::Proxy { headers, .. }) => {
                        headers.insert(header_name, header_value);
                    }
                    Some(_) => return Err(ParseError::DuplicateContent(route.uri)),
                }
            }
            "header" => {
                let header_name =
                    replace_static_vars(&next_token(reader)?, &[&vars, server_vars, global_vars])
                        .unwrap();
                let header_value =
                    replace_static_vars(&next_token(reader)?, &[&vars, server_vars, global_vars])
                        .unwrap();
                consume_fixed(reader, "\n")?;

                route.headers.insert(header_name, header_value);
            }
            "set" => {
                let (var, value) = parse_set(reader, &[&vars, server_vars, global_vars])?;
                vars.insert(var, value);
            }
            "\n" => {}
            "}" => break,
            "" => return Err(ParseError::UnexpectedEof),
            _ => return Err(ParseError::UnknownField(token)),
        }
    }
    Ok(route)
}

fn parse_server_config<Reader: BufRead>(
    reader: &mut Reader,
    global_vars: &HashMap<String, String>,
) -> Result<Server, ParseError> {
    consume_fixed(reader, "{")?;

    let mut server = Server::default();
    let mut vars = HashMap::new();

    loop {
        let token = next_token(reader)?;
        match token.as_str() {
            "port" => {
                let port = replace_static_vars(&next_token(reader)?, &[&vars, global_vars])
                    .unwrap()
                    .parse()
                    .map_err(ParseError::ParseIntError)?;

                consume_fixed(reader, "\n")?;

                if server.port.is_none() {
                    server.port = Some(port)
                } else {
                    return Err(ParseError::DuplicateField(token));
                }
            }
            "ip" => {
                let ip = replace_static_vars(&next_token(reader)?, &[&vars, global_vars])
                    .unwrap()
                    .as_str()
                    .parse()
                    .map_err(ParseError::AddrParseError)?;

                consume_fixed(reader, "\n")?;

                if server.ip.is_none() {
                    server.ip = Some(ip)
                } else {
                    return Err(ParseError::DuplicateField(token));
                }
            }
            "domain" => {
                let mut domains = read_tokens_until_newline(reader)?;
                server.domain_names.append(&mut domains);
            }
            "default" => {
                consume_fixed(reader, "\n")?;

                if server.is_default {
                    return Err(ParseError::DuplicateField(token));
                } else {
                    server.is_default = true;
                }
            }
            "route" => {
                server.routes.push(parse_route(reader, global_vars, &vars)?);
                consume_fixed(reader, "\n")?;
            }
            "header_timeout" => {
                let timeout = Duration::from_secs(
                    next_token(reader)?
                        .parse()
                        .map_err(ParseError::ParseIntError)?,
                );

                if server.header_timeout.is_none() {
                    server.header_timeout = Some(timeout);
                } else {
                    return Err(ParseError::DuplicateField(token));
                }
            }
            "body_timeout" => {
                let timeout = Duration::from_secs(
                    next_token(reader)?
                        .parse()
                        .map_err(ParseError::ParseIntError)?,
                );

                if server.body_timeout.is_none() {
                    server.body_timeout = Some(timeout);
                } else {
                    return Err(ParseError::DuplicateField(token));
                }
            }
            "send_timeout" => {
                let timeout = Duration::from_secs(
                    next_token(reader)?
                        .parse()
                        .map_err(ParseError::ParseIntError)?,
                );

                if server.send_timeout.is_none() {
                    server.send_timeout = Some(timeout);
                } else {
                    return Err(ParseError::DuplicateField(token));
                }
            }
            "ssl_cert" => {
                let cert_path = PathBuf::from(next_token(reader)?);

                if server.cert_path.is_none() {
                    server.cert_path = Some(cert_path);
                } else {
                    return Err(ParseError::DuplicateField(token));
                }
            }
            "ssl_keys" => {
                let keys_path = PathBuf::from(next_token(reader)?);

                if server.keys_path.is_none() {
                    server.keys_path = Some(keys_path);
                } else {
                    return Err(ParseError::DuplicateField(token));
                }
            }
            "set" => {
                let (var, value) = parse_set(reader, &[&vars, global_vars])?;
                vars.insert(var, value);
            }
            "\n" => {}
            "}" => break,
            "" => return Err(ParseError::UnexpectedEof),
            _ => return Err(ParseError::UnknownField(token)),
        }
    }

    if server.cert_path.is_some() && server.keys_path.is_none() {
        return Err(ParseError::MissingField("key_path".to_owned()));
    } else if server.cert_path.is_none() && server.keys_path.is_some() {
        return Err(ParseError::MissingField("cert_path".to_owned()));
    }

    Ok(server)
}

pub fn parse_upstream_config<Reader: BufRead>(
    reader: &mut Reader,
    global_vars: &HashMap<String, String>,
) -> Result<(String, Upstream), ParseError> {
    let name = next_token(reader)?;
    consume_fixed(reader, "{")?;

    let mut upstream = Upstream::default();
    let mut vars = HashMap::new();

    loop {
        let token = next_token(reader)?;
        match token.as_str() {
            "server" => {
                let uri = replace_static_vars(&next_token(reader)?, &[&vars, global_vars]).unwrap();
                upstream.servers.push(uri);
            }
            "round_robin" => {
                if upstream.balancing.is_none() {
                    upstream.balancing = Some(Balancing::RoundRobin);
                } else {
                    return Err(ParseError::DuplicateBalancing(name));
                }
            }
            "least_conn" => {
                if upstream.balancing.is_none() {
                    upstream.balancing = Some(Balancing::LeastConnected);
                } else {
                    return Err(ParseError::DuplicateBalancing(name));
                }
            }
            "ip_hash" => {
                if upstream.balancing.is_none() {
                    upstream.balancing = Some(Balancing::IpHash);
                } else {
                    return Err(ParseError::DuplicateBalancing(name));
                }
            }
            "connect_timeout" => {
                let timeout = Duration::from_secs(
                    next_token(reader)?
                        .parse()
                        .map_err(ParseError::ParseIntError)?,
                );

                if upstream.connect_timeout.is_none() {
                    upstream.connect_timeout = Some(timeout);
                } else {
                    return Err(ParseError::DuplicateField(token));
                }
            }
            "header_timeout" => {
                let timeout = Duration::from_secs(
                    next_token(reader)?
                        .parse()
                        .map_err(ParseError::ParseIntError)?,
                );

                if upstream.header_timeout.is_none() {
                    upstream.header_timeout = Some(timeout);
                } else {
                    return Err(ParseError::DuplicateField(token));
                }
            }
            "send_timeout" => {
                let timeout = Duration::from_secs(
                    next_token(reader)?
                        .parse()
                        .map_err(ParseError::ParseIntError)?,
                );

                if upstream.send_timeout.is_none() {
                    upstream.send_timeout = Some(timeout);
                } else {
                    return Err(ParseError::DuplicateField(token));
                }
            }
            "set" => {
                let (var, value) = parse_set(reader, &[&vars, global_vars])?;
                vars.insert(var, value);
            }
            "\n" => {}
            "}" => break,
            "" => return Err(ParseError::UnexpectedEof),
            _ => return Err(ParseError::UnknownField(token)),
        }
    }

    Ok((name, upstream))
}

pub fn parse_config<Reader: BufRead>(reader: &mut Reader) -> Result<Config, ParseError> {
    let mut config = Config::default();
    let mut vars = HashMap::new();

    loop {
        let token = next_token(reader)?;
        match token.as_str() {
            "server" => config.servers.push(parse_server_config(reader, &vars)?),
            "upstream" => {
                let (name, upstream) = parse_upstream_config(reader, &vars)?;
                config.upstreams.insert(name, upstream);
            }
            "set" => {
                let (var, value) = parse_set(reader, &[&vars])?;
                vars.insert(var, value);
            }
            "\n" => {}
            "" => break,
            _ => return Err(ParseError::UnknownField(token)),
        }
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    use crate::config::{
        Balancing, Config, Content, ParseError, Route, Server, Upstream, UriMatcher,
    };
    use indoc::indoc;
    use std::{collections::HashMap, io::Cursor, net::Ipv4Addr};

    #[test]
    fn parse_config() -> Result<(), ParseError> {
        let mut reader = Cursor::new(indoc! {"
            set port 8080

            server {
                port $port
                default

                set index_uri /index.html

                route / {
                    redirect $index_uri
                }

                route $index_uri {
                    file /index.html /index.htm \"/has a space.html\"

                    set type text/html
                    header Content-Type $type
                }

                route /images/ {}

                route /test {
                    body \"test test test\"
                }

                route /proxy {
                    proxy http://example.com
                    proxy_header Host $host
                }
            }

            # a comment

            upstream backend {
                least_conn
                server app1.example.com
                server app2.example.com
                server app3.example.com
            }

            server {
                port $port
                ip 127.0.0.1
                domain example.com
                domain www.example.com

                route = /blocked {
                    nocontent
                    status 404
                }
            }
        "});

        assert_eq!(
            super::parse_config(&mut reader)?,
            Config {
                servers: Vec::from([
                    Server {
                        port: Some(8080),
                        ip: None,
                        domain_names: Vec::new(),
                        is_default: true,
                        routes: Vec::from([
                            Route {
                                uri: "/".to_owned(),
                                matcher: UriMatcher::Prefix,
                                content: Some(Content::Redirect("/index.html".to_owned())),
                                status: None,
                                headers: HashMap::new(),
                            },
                            Route {
                                uri: "/index.html".to_owned(),
                                matcher: UriMatcher::Prefix,
                                content: Some(Content::FileAny(Vec::from([
                                    "/index.html".into(),
                                    "/index.htm".into(),
                                    "/has a space.html".into(),
                                ]))),
                                status: None,
                                headers: HashMap::from(HashMap::from([(
                                    "Content-Type".to_owned(),
                                    "text/html".into()
                                )])),
                            },
                            Route {
                                uri: "/images/".to_owned(),
                                matcher: UriMatcher::Prefix,
                                content: None,
                                status: None,
                                headers: HashMap::new()
                            },
                            Route {
                                uri: "/test".to_owned(),
                                matcher: UriMatcher::Prefix,
                                content: Some(Content::RawData("test test test".to_owned())),
                                status: None,
                                headers: HashMap::new(),
                            },
                            Route {
                                uri: "/proxy".to_owned(),
                                matcher: UriMatcher::Prefix,
                                content: Some(Content::Proxy {
                                    uri: "http://example.com".to_owned(),
                                    headers: HashMap::from([(
                                        "Host".to_owned(),
                                        "$host".to_owned()
                                    )])
                                }),
                                status: None,
                                headers: HashMap::new(),
                            }
                        ]),
                        header_timeout: None,
                        body_timeout: None,
                        send_timeout: None,
                        cert_path: None,
                        keys_path: None,
                    },
                    Server {
                        port: Some(8080),
                        ip: Some(Ipv4Addr::from_octets([127, 0, 0, 1])),
                        domain_names: Vec::from([
                            "example.com".to_owned(),
                            "www.example.com".to_owned()
                        ]),
                        is_default: false,
                        routes: Vec::from([Route {
                            uri: "/blocked".to_owned(),
                            matcher: UriMatcher::Exact,
                            content: Some(Content::NoContent),
                            status: Some(404),
                            headers: HashMap::new(),
                        }]),
                        header_timeout: None,
                        body_timeout: None,
                        send_timeout: None,
                        cert_path: None,
                        keys_path: None
                    }
                ]),
                upstreams: HashMap::from([(
                    "backend".to_owned(),
                    Upstream {
                        balancing: Some(Balancing::LeastConnected),
                        servers: Vec::from([
                            "app1.example.com".to_owned(),
                            "app2.example.com".to_owned(),
                            "app3.example.com".to_owned(),
                        ]),
                        max_fails: None,
                        fail_timeout: None,
                        connect_timeout: None,
                        header_timeout: None,
                        body_timeout: None,
                        send_timeout: None,
                    },
                )]),
            }
        );

        Ok(())
    }
}
