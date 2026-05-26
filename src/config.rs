use std::{
    collections::HashMap,
    io::{self, BufRead},
    num::ParseIntError,
    path::PathBuf,
    string::FromUtf8Error,
};

use thiserror::Error;

#[derive(Debug, Default, PartialEq)]
pub enum Content {
    #[default]
    NoContent,
    FileAny(Vec<PathBuf>),
    Redirect(String),
}

#[derive(Debug, Default, PartialEq)]
pub struct Route {
    pub uri: String,
    pub content: Content,
    pub status: Option<i16>,
    pub headers: HashMap<String, Vec<u8>>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Server {
    pub port: Option<i16>,
    pub ip: Option<i32>,
    pub domain_names: Vec<String>,
    pub is_default: bool,
    pub routes: Vec<Route>,
}

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Io error: {0}")]
    IoError(io::Error),
    #[error("Utf8 error: {0}")]
    Utf8Error(FromUtf8Error),
    #[error("Parse int error: {0}")]
    ParseIntError(ParseIntError),
    #[error("Unexpected token: {0}, expected {1}")]
    UnexpectedToken(String, String),
    #[error("Unexpected eof")]
    UnexpectedEof,
    #[error("Unknown field: {0}")]
    UnknownField(String),
    #[error("Duplicate field: {0}")]
    DuplicateField(String),
    #[error("IPv4 address should have 4 octets")]
    IpNotEnoughOctets,
    #[error("Route {0} already has content")]
    DuplicateContent(String),
}

fn skip_whitespace<Reader: BufRead>(reader: &mut Reader) -> Result<(), ParseError> {
    loop {
        let buf = reader.fill_buf().map_err(ParseError::IoError)?;
        if buf.is_empty() {
            return Ok(());
        }

        for (i, c) in buf.iter().copied().enumerate() {
            if !c.is_ascii_whitespace() {
                reader.consume(i);
                return Ok(());
            }
        }

        let len = buf.len();
        reader.consume(len);
    }
}

fn next_token<Reader: BufRead>(reader: &mut Reader) -> Result<String, ParseError> {
    skip_whitespace(reader)?;
    let mut token = Vec::new();

    'fill_buf: loop {
        let buf = reader.fill_buf().map_err(ParseError::IoError)?;
        if buf.is_empty() {
            break;
        }

        for (i, c) in buf.iter().copied().enumerate() {
            if c.is_ascii_whitespace() {
                reader.consume(i);
                break 'fill_buf;
            }

            token.push(c);
        }

        let len = buf.len();
        reader.consume(len);
    }

    String::from_utf8(token).map_err(ParseError::Utf8Error)
}

fn consume_fixed<Reader: BufRead>(reader: &mut Reader, expected: &str) -> Result<(), ParseError> {
    let token = next_token(reader)?;
    if token != expected {
        Err(ParseError::UnexpectedToken(token, expected.to_owned()))
    } else {
        Ok(())
    }
}

fn parse_ip(ip: &str) -> Result<i32, ParseError> {
    let mut result = 0;
    let mut octet_start = 0;
    for _ in 0..3 {
        if let Some(octet_len) = ip[octet_start..].find('.') {
            result += ip[octet_start..octet_start+octet_len]
                .parse::<i8>()
                .map_err(ParseError::ParseIntError)? as i32;
            result <<= 8;

            octet_start = octet_start + octet_len + 1;
        } else {
            return Err(ParseError::IpNotEnoughOctets);
        }
    }

    result += ip[octet_start..]
        .parse::<i8>()
        .map_err(ParseError::ParseIntError)? as i32;
    Ok(result)
}

fn parse_route<Reader: BufRead>(reader: &mut Reader) -> Result<Route, ParseError> {
    let mut route = Route::default();
    route.uri = next_token(reader)?;
    consume_fixed(reader, "{")?;

    loop {
        let token = next_token(reader)?;
        match token.as_str() {
            "status" => {
                let status = next_token(reader)?
                    .parse()
                    .map_err(ParseError::ParseIntError)?;

                if route.status.is_none() {
                    route.status = Some(status)
                } else {
                    return Err(ParseError::DuplicateField(token));
                }
            }
            "file" => {
                let path = next_token(reader)?.into();

                match route.content {
                    Content::NoContent => route.content = Content::FileAny(Vec::from([path])),
                    Content::FileAny(mut files) => {
                        files.push(path);
                        route.content = Content::FileAny(files);
                    }
                    Content::Redirect(_) => return Err(ParseError::DuplicateContent(route.uri)),
                }
            }
            "redirect" => {
                let uri = next_token(reader)?;

                match route.content {
                    Content::NoContent => route.content = Content::Redirect(uri),
                    _ => return Err(ParseError::DuplicateContent(route.uri)),
                }
            }
            "header" => {
                let header_name = next_token(reader)?;
                let header_value = next_token(reader)?;

                route.headers.insert(header_name, header_value.into_bytes());
            }
            "}" => break,
            "" => return Err(ParseError::UnexpectedEof),
            _ => return Err(ParseError::UnknownField(token)),
        }
    }
    Ok(route)
}

fn parse_server_config<Reader: BufRead>(reader: &mut Reader) -> Result<Server, ParseError> {
    consume_fixed(reader, "{")?;

    let mut server = Server::default();
    loop {
        let token = next_token(reader)?;
        match token.as_str() {
            "port" => {
                let port = next_token(reader)?
                    .parse()
                    .map_err(ParseError::ParseIntError)?;

                if server.port.is_none() {
                    server.port = Some(port)
                } else {
                    return Err(ParseError::DuplicateField(token));
                }
            }
            "ip" => {
                let ip = parse_ip(next_token(reader)?.as_str())?;

                if server.ip.is_none() {
                    server.ip = Some(ip)
                } else {
                    return Err(ParseError::DuplicateField(token));
                }
            }
            "domain" => {
                let domain = next_token(reader)?;
                server.domain_names.push(domain);
            }
            "default" => {
                if server.is_default {
                    return Err(ParseError::DuplicateField(token));
                } else {
                    server.is_default = true;
                }
            }
            "route" => {
                server.routes.push(parse_route(reader)?);
            }
            "}" => break,
            "" => return Err(ParseError::UnexpectedEof),
            _ => return Err(ParseError::UnknownField(token)),
        }
    }
    Ok(server)
}

pub fn parse_config<Reader: BufRead>(reader: &mut Reader) -> Result<Vec<Server>, ParseError> {
    let mut servers = Vec::new();

    loop {
        let token = next_token(reader)?;
        match token.as_str() {
            "server" => servers.push(parse_server_config(reader)?),
            "" => break,
            _ => return Err(ParseError::UnknownField(token)),
        }
    }

    Ok(servers)
}

#[cfg(test)]
mod tests {
    use crate::config::{self, Content, Route, Server};
    use indoc::indoc;
    use std::{collections::HashMap, io::Cursor};

    #[test]
    fn parse_config() -> Result<(), config::ParseError> {
        let mut reader = Cursor::new(indoc! {"
            server {
                port 8080
                default

                route / {
                    redirect /index.html
                }

                route /index.html {
                    file /index.html
                    file /index.htm
                    header Content-Type text/html
                }
            }

            server {
                port 8080
                ip 127.0.0.1
                domain example.com
                domain www.example.com

                route /blocked {
                    status 404
                }
            }
        "});

        assert_eq!(
            config::parse_config(&mut reader)?,
            Vec::from([
                Server {
                    port: Some(8080),
                    ip: None,
                    domain_names: Vec::new(),
                    is_default: true,
                    routes: Vec::from([
                        Route {
                            uri: "/".to_owned(),
                            content: Content::Redirect("/index.html".to_owned()),
                            status: None,
                            headers: HashMap::new(),
                        },
                        Route {
                            uri: "/index.html".to_owned(),
                            content: Content::FileAny(Vec::from([
                                "/index.html".into(),
                                "/index.htm".into()
                            ])),
                            status: None,
                            headers: HashMap::from(HashMap::from([(
                                "Content-Type".to_owned(),
                                "text/html".into()
                            )])),
                        },
                    ]),
                },
                Server {
                    port: Some(8080),
                    ip: Some((127 << 24) + 1),
                    domain_names: Vec::from([
                        "example.com".to_owned(),
                        "www.example.com".to_owned()
                    ]),
                    is_default: false,
                    routes: Vec::from([Route {
                        uri: "/blocked".to_owned(),
                        content: Content::NoContent,
                        status: Some(404),
                        headers: HashMap::new(),
                    }])
                }
            ]),
        );

        Ok(())
    }
}
