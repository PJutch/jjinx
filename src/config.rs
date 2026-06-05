use std::{
    collections::HashMap,
    io::{self, BufRead},
    net::{AddrParseError, Ipv4Addr},
    num::ParseIntError,
    string::FromUtf8Error,
};

use thiserror::Error;

#[derive(Debug, Default, PartialEq)]
pub enum Content {
    #[default]
    NoContent,
    RawData(String),
    FileAny(Vec<String>),
    Redirect(String),
}

#[derive(Debug, Default, PartialEq)]
pub enum UriMatcher {
    #[default]
    Prefix,
    Regex,
    PrefixPrioritiesed,
    Exact,
}

#[derive(Debug, Default, PartialEq)]
pub struct Route {
    pub uri: String,
    pub matcher: UriMatcher,
    pub content: Option<Content>,
    pub status: Option<i16>,
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Server {
    pub port: Option<u16>,
    pub ip: Option<Ipv4Addr>,
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
    #[error("Address parse error: {0}")]
    AddrParseError(AddrParseError),
    #[error("Unexpected token: {0}, expected {1}")]
    UnexpectedToken(String, String),
    #[error("Unexpected eof")]
    UnexpectedEof,
    #[error("Unknown field: {0}")]
    UnknownField(String),
    #[error("Duplicate field: {0}")]
    DuplicateField(String),
    #[error("Route {0} already has content")]
    DuplicateContent(String),
    #[error("Unknown matcher: {0}")]
    UnknownMatcher(String),
}

fn skip_whitespace<Reader: BufRead>(reader: &mut Reader) -> Result<(), ParseError> {
    loop {
        let buf = reader.fill_buf().map_err(ParseError::IoError)?;
        if buf.is_empty() {
            return Ok(());
        }

        for (i, c) in buf.iter().copied().enumerate() {
            if !c.is_ascii_whitespace() || c == '\n' as u8 {
                reader.consume(i);
                return Ok(());
            }
        }

        let len = buf.len();
        reader.consume(len);
    }
}

#[derive(PartialEq)]
enum TokenizerMode {
    NORMAL,
    QUOTED,
    COMMENT,
}

fn next_token<Reader: BufRead>(reader: &mut Reader) -> Result<String, ParseError> {
    skip_whitespace(reader)?;

    let mut token = Vec::new();
    let mut mode = TokenizerMode::NORMAL;

    'fill_buf: loop {
        let buf = reader.fill_buf().map_err(ParseError::IoError)?;
        if buf.is_empty() {
            break;
        }

        for (i, c) in buf.iter().copied().enumerate() {
            if mode == TokenizerMode::QUOTED {
                if c == '"' as u8 {
                    reader.consume(i + 1);
                    break 'fill_buf;
                } else {
                    token.push(c);
                }
            } else if mode == TokenizerMode::COMMENT {
                if c == '\n' as u8 {
                    if token.is_empty() {
                        token.push(c);
                        reader.consume(i + 1);
                    } else {
                        reader.consume(i);
                    }
                    break 'fill_buf;
                }
            } else {
                if c == '#' as u8 {
                    mode = TokenizerMode::COMMENT;
                } else if c == '"' as u8 && token.is_empty() {
                    mode = TokenizerMode::QUOTED;
                } else if c == '"' as u8 && !token.is_empty() {
                    reader.consume(i);
                    break 'fill_buf;
                } else if c == '\n' as u8 && token.is_empty() {
                    token.push(c);
                    reader.consume(i + 1);
                    break 'fill_buf;
                } else if c.is_ascii_whitespace() {
                    reader.consume(i);
                    break 'fill_buf;
                } else if c == '}' as u8 && !token.is_empty() {
                    reader.consume(i);
                    break 'fill_buf;
                } else {
                    token.push(c);
                }
            }
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

fn read_tokens_until_newline<Reader: BufRead>(
    reader: &mut Reader,
) -> Result<Vec<String>, ParseError> {
    let mut result = Vec::new();
    loop {
        let token = next_token(reader)?;
        if token == "\n" {
            break;
        }
        result.push(token);
    }
    Ok(result)
}

fn parse_route<Reader: BufRead>(reader: &mut Reader) -> Result<Route, ParseError> {
    let mut route = Route::default();

    let token1 = next_token(reader)?;
    let token2 = next_token(reader)?;

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

    loop {
        let token = next_token(reader)?;
        match token.as_str() {
            "status" => {
                let status = next_token(reader)?
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
                let body = next_token(reader)?;
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
                    .map(|path| path.into())
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
                let uri = next_token(reader)?;
                consume_fixed(reader, "\n")?;

                if route.content.is_none() {
                    route.content = Some(Content::Redirect(uri));
                } else {
                    return Err(ParseError::DuplicateContent(route.uri));
                }
            }
            "header" => {
                let header_name = next_token(reader)?;
                let header_value = next_token(reader)?;
                consume_fixed(reader, "\n")?;

                route.headers.insert(header_name, header_value);
            }
            "\n" => {}
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

                consume_fixed(reader, "\n")?;

                if server.port.is_none() {
                    server.port = Some(port)
                } else {
                    return Err(ParseError::DuplicateField(token));
                }
            }
            "ip" => {
                let ip = next_token(reader)?
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
                server.routes.push(parse_route(reader)?);
                consume_fixed(reader, "\n")?;
            }
            "\n" => {}
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
            "\n" => {}
            "" => break,
            _ => return Err(ParseError::UnknownField(token)),
        }
    }

    Ok(servers)
}

#[cfg(test)]
mod tests {
    use crate::config::{self, Content, Route, Server, UriMatcher};
    use indoc::indoc;
    use std::{collections::HashMap, io::Cursor, net::Ipv4Addr};

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
                    file /index.html /index.htm \"/has a space.html\"
                    header Content-Type text/html
                }

                route /images/ {}

                route /test {
                    body \"test test test\"
                }
            }

            # a comment

            server {
                port 8080
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
                        }
                    ]),
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
                    }])
                }
            ]),
        );

        Ok(())
    }
}
