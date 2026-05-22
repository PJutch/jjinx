use std::io;
use std::num::ParseIntError;
use std::{collections::HashMap, str::Utf8Error};
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Unexpected eof")]
    UnexpectedEof,
    #[error("Io error: {0}")]
    IoError(io::Error),
    #[error("Utf8 error: {0}")]
    Utf8Error(Utf8Error),
    #[error("Expected a digit")]
    ExpectedDigit,
    #[error("Parse int error: {0}")]
    ParseIntError(ParseIntError),
    #[error("Invalid HTTP version")]
    InvalidVersion,
    #[error("Expected space")]
    ExpectedSpace,
    #[error("Uri parse error")]
    UriParseError,
    #[error("Expected newline")]
    ExpectedNewline,
    #[error("Expected colon after field name")]
    ExpectedColon,
    #[error("Expected line feed after carraige return")]
    ExpectedLineFeed,
}

async fn try_read_byte<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
) -> Result<Option<u8>, ParseError> {
    let buf = reader.fill_buf().await.map_err(ParseError::IoError)?;
    let c = buf.first().copied();
    if !buf.is_empty() {
        reader.consume(1);
    }
    Ok(c)
}

async fn read_byte<Reader: AsyncBufRead + Unpin>(reader: &mut Reader) -> Result<u8, ParseError> {
    try_read_byte(reader)
        .await?
        .ok_or(ParseError::UnexpectedEof)
}

async fn peek_byte<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
) -> Result<Option<u8>, ParseError> {
    let buf = reader.fill_buf().await.map_err(ParseError::IoError)?;
    Ok(buf.first().copied())
}

async fn read_digit<Reader: AsyncBufRead + Unpin>(reader: &mut Reader) -> Result<u8, ParseError> {
    let c = read_byte(reader).await?;
    if c.is_ascii_digit() {
        Ok(c - ('0' as u8))
    } else {
        Err(ParseError::ExpectedDigit)
    }
}

async fn skip_fixed<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
    str: &str,
) -> Result<bool, ParseError> {
    let mut i = 0;
    while i < str.len() {
        let buf = reader.fill_buf().await.map_err(ParseError::IoError)?;
        if buf.is_empty() {
            return Err(ParseError::UnexpectedEof);
        }

        for (j, c) in buf.iter().copied().enumerate() {
            if i + j >= str.len() {
                reader.consume(j);
                return Ok(true);
            }

            if c != str.as_bytes()[i + j] {
                reader.consume(j);
                return Ok(false);
            }
        }

        let len = buf.len();
        i += len;
        reader.consume(len);
    }
    Ok(true)
}

#[derive(PartialEq, Debug)]
pub struct HttpVersion(u8, u8);

async fn parse_http_version<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
) -> Result<HttpVersion, ParseError> {
    skip_fixed(reader, "HTTP/").await.and_then(|matched| {
        if matched {
            Ok(())
        } else {
            Err(ParseError::InvalidVersion)
        }
    })?;

    let digit1 = read_digit(reader).await?;

    if read_byte(reader).await? != '.' as u8 {
        return Err(ParseError::InvalidVersion);
    }

    let digit2 = read_digit(reader).await?;

    Ok(HttpVersion(digit1, digit2))
}

fn is_tchar(c: u8) -> bool {
    c.is_ascii_alphanumeric()
        || [
            '!', '#', '$', '%', '&', '\'', '*', '+', '-', '.', '^', '_', '`', '|', '~',
        ]
        .contains(&(c as char))
}

async fn read_token<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
) -> Result<String, ParseError> {
    let mut result = String::new();
    loop {
        if let Some(c) = peek_byte(reader).await? {
            if is_tchar(c) {
                result.push(c as char);
                reader.consume(1);
            } else {
                break;
            }
        } else {
            break;
        }
    }
    Ok(result)
}

#[derive(Default, PartialEq, Debug)]
struct Uri {
    scheme: String,
    authority: String,
    path: Vec<String>,
    path_absolute: bool,
    query: String,
    fragment: String,
}

fn is_uri_char(c: u8) -> bool {
    c.is_ascii_alphanumeric()
        || [
            '-', '.', '_', '~', ',', '[', ']', '!', '%', '$', '&', ':', '@', '\'', '(', ')', '*',
            '+', ',', ';', '=', '#', '?', '/',
        ]
        .contains(&(c as char))
}

#[derive(PartialEq, Eq)]
enum UriParseStep {
    Scheme,
    Authority,
    Path,
    Query,
    Fragment,
}

async fn parse_uri<Reader: AsyncBufRead + Unpin>(reader: &mut Reader) -> Result<Uri, ParseError> {
    let mut result = Uri::default();
    let mut current = String::new();
    let mut step = UriParseStep::Scheme;

    if peek_byte(reader).await? == Some('/' as u8) {
        result.path_absolute = true;
        reader.consume(1);

        if peek_byte(reader).await? == Some('/' as u8) {
            step = UriParseStep::Authority;
            reader.consume(1);
        } else {
            step = UriParseStep::Path;
        }
    }

    loop {
        if let Some(c) = peek_byte(reader).await? {
            if !is_uri_char(c) {
                break;
            }
            reader.consume(1);

            if c == ':' as u8 && step == UriParseStep::Scheme {
                result.scheme = std::mem::take(&mut current);

                if peek_byte(reader).await? == Some('/' as u8) {
                    result.path_absolute = true;
                    reader.consume(1);

                    if peek_byte(reader).await? == Some('/' as u8) {
                        step = UriParseStep::Authority;
                        reader.consume(1);
                    } else {
                        step = UriParseStep::Path;
                    }
                } else {
                    step = UriParseStep::Path;
                }
            } else if c == '/' as u8 {
                if step == UriParseStep::Scheme {
                    result.path.push(std::mem::take(&mut current));
                    step = UriParseStep::Path;
                } else if step == UriParseStep::Authority {
                    result.authority = std::mem::take(&mut current);
                    result.path_absolute = true;
                    step = UriParseStep::Path;
                } else if step == UriParseStep::Path {
                    result.path.push(std::mem::take(&mut current));
                }
            } else if c == '?' as u8 {
                match step {
                    UriParseStep::Scheme | UriParseStep::Authority => {
                        result.authority = std::mem::take(&mut current)
                    }
                    UriParseStep::Path => {
                        if !current.is_empty() {
                            result.path.push(std::mem::take(&mut current));
                        }
                    }
                    UriParseStep::Query => return Err(ParseError::UriParseError),
                    UriParseStep::Fragment => current.push('?'),
                }

                if step != UriParseStep::Fragment {
                    step = UriParseStep::Query;
                }
            } else if c == '#' as u8 && step != UriParseStep::Fragment {
                match step {
                    UriParseStep::Scheme | UriParseStep::Authority => {
                        result.authority = std::mem::take(&mut current)
                    }
                    UriParseStep::Path => {
                        if !current.is_empty() {
                            result.path.push(std::mem::take(&mut current))
                        }
                    }
                    UriParseStep::Query => result.query = std::mem::take(&mut current),
                    UriParseStep::Fragment => return Err(ParseError::UriParseError),
                }
                step = UriParseStep::Fragment;
            } else {
                current.push(c as char);
            }
        } else {
            break;
        }
    }

    match step {
        UriParseStep::Scheme | UriParseStep::Authority => {
            result.authority = std::mem::take(&mut current);
        }
        UriParseStep::Path => {
            result.path.push(std::mem::take(&mut current));
        }
        UriParseStep::Query => {
            result.query = std::mem::take(&mut current);
        }
        UriParseStep::Fragment => {
            result.fragment = std::mem::take(&mut current);
        }
    }

    Ok(result)
}

#[derive(PartialEq, Debug)]
struct StartLine {
    method: String,
    uri: Uri,
    version: HttpVersion,
}

async fn parse_start_line<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
) -> Result<StartLine, ParseError> {
    let method = read_token(reader).await?;
    if read_byte(reader).await? != ' ' as u8 {
        return Err(ParseError::ExpectedSpace);
    }

    let uri = parse_uri(reader).await?;
    if read_byte(reader).await? != ' ' as u8 {
        return Err(ParseError::ExpectedSpace);
    }

    let version = parse_http_version(reader).await?;

    Ok(StartLine {
        method,
        uri,
        version,
    })
}

async fn try_skip_newline<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
) -> Result<bool, ParseError> {
    match peek_byte(reader).await? {
        Some(0xD) => {
            reader.consume(1);
            if read_byte(reader).await? != '\n' as u8 {
                return Err(ParseError::ExpectedLineFeed);
            }
            Ok(true)
        }
        Some(_) => Err(ParseError::ExpectedNewline),
        None => Ok(false),
    }
}

fn is_whitespace(byte: u8) -> bool {
    byte == ' ' as u8 || byte == '\t' as u8
}

async fn skip_whitespace<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
) -> Result<(), ParseError> {
    loop {
        if peek_byte(reader).await?.is_some_and(is_whitespace) {
            reader.consume(1);
        } else {
            return Ok(());
        }
    }
}

async fn read_until<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
    until: u8,
) -> Result<Vec<u8>, ParseError> {
    let mut result = Vec::new();
    loop {
        if let Some(byte) = peek_byte(reader).await? {
            if byte == until {
                return Ok(result);
            } else {
                result.push(byte);
                reader.consume(1);
            }
        } else {
            return Ok(result);
        }
    }
}

async fn parse_headers<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
) -> Result<HashMap<String, Vec<u8>>, ParseError> {
    let mut headers = HashMap::new();
    loop {
        let first_byte = peek_byte(reader).await?;
        if first_byte == Some('\r' as u8) || first_byte == None {
            return Ok(headers);
        }

        let field_name = read_token(reader).await?;
        if read_byte(reader).await? != ':' as u8 {
            return Err(ParseError::ExpectedColon);
        }

        skip_whitespace(reader).await?;
        let mut field_value = read_until(reader, '\r' as u8).await?;
        while field_value.last().copied().is_some_and(is_whitespace) {
            field_value.pop();
        }

        headers.insert(field_name, field_value);
        try_skip_newline(reader).await?;
    }
}

async fn read_n_bytes<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
    n: usize,
) -> Result<Vec<u8>, ParseError> {
    let mut result = Vec::new();
    while result.len() < n {
        let mut buf = reader.fill_buf().await.map_err(ParseError::IoError)?;
        if buf.is_empty() {
            return Ok(result);
        }

        if buf.len() > n - result.len() {
            buf = &buf[..n - result.len()]
        }

        result.extend_from_slice(buf);

        let len = buf.len();
        reader.consume(len);
    }
    Ok(result)
}

#[derive(PartialEq, Debug)]
pub struct Request {
    start_line: StartLine,
    headers: HashMap<String, Vec<u8>>,
    body: Vec<u8>,
}

pub async fn parse_request<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
) -> Result<Request, ParseError> {
    let start_line = parse_start_line(reader).await?;
    try_skip_newline(reader).await?;
    let headers = parse_headers(reader).await?;
    try_skip_newline(reader).await?;

    let body = if let Some(data) = headers.get("Content-Length") {
        let len = str::from_utf8(data)
            .map_err(ParseError::Utf8Error)?
            .parse()
            .map_err(ParseError::ParseIntError)?;
        read_n_bytes(reader, len).await?
    } else if let Some(_) = headers.get("Transfer-Encoding") {
        todo!("Handle Transfer-Encoding");
    } else {
        Vec::new()
    };

    Ok(Request {
        start_line,
        headers,
        body,
    })
}

#[cfg(test)]
mod tests {
    use crate::parse::{
        self, HttpVersion, ParseError, Request, StartLine, Uri, parse_request, parse_uri,
    };
    use indoc::indoc;
    use std::{collections::HashMap, io::Cursor};

    #[tokio::test]
    async fn parse_uri_full() -> Result<(), ParseError> {
        let mut reader = Cursor::new("http://www.ics.uci.edu/pub/ietf/uri/#Related");
        let uri = parse_uri(&mut reader).await?;
        assert_eq!(
            uri,
            Uri {
                scheme: "http".to_owned(),
                authority: "www.ics.uci.edu".to_owned(),
                path: ["pub", "ietf", "uri"].map(&str::to_owned).to_vec(),
                path_absolute: true,
                query: "".to_owned(),
                fragment: "Related".to_owned()
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn parse_uri_path_only() -> Result<(), ParseError> {
        let mut reader = Cursor::new("/pub/ietf/uri/#Related");
        let uri = parse_uri(&mut reader).await?;
        assert_eq!(
            uri,
            Uri {
                scheme: "".to_owned(),
                authority: "".to_owned(),
                path: ["pub", "ietf", "uri"].map(&str::to_owned).to_vec(),
                path_absolute: true,
                query: "".to_owned(),
                fragment: "Related".to_owned()
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn parse_uri_relative() -> Result<(), ParseError> {
        let mut reader = Cursor::new("pub/ietf/uri/#Related");
        let uri = parse_uri(&mut reader).await?;
        assert_eq!(
            uri,
            Uri {
                scheme: "".to_owned(),
                authority: "".to_owned(),
                path: ["pub", "ietf", "uri"].map(&str::to_owned).to_vec(),
                path_absolute: false,
                query: "".to_owned(),
                fragment: "Related".to_owned()
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn parse_http_version() -> Result<(), ParseError> {
        let mut reader = Cursor::new("HTTP/1.1");
        let version = parse::parse_http_version(&mut reader).await?;
        assert_eq!(version, HttpVersion(1, 1));
        Ok(())
    }

    #[tokio::test]
    async fn parse_start_line() -> Result<(), ParseError> {
        let mut reader = Cursor::new("GET http://www.ics.uci.edu/pub/ietf/uri/#Related HTTP/1.1");
        let start_line = parse::parse_start_line(&mut reader).await?;
        assert_eq!(
            start_line,
            StartLine {
                method: "GET".to_owned(),
                uri: Uri {
                    scheme: "http".to_owned(),
                    authority: "www.ics.uci.edu".to_owned(),
                    path: ["pub", "ietf", "uri"].map(&str::to_owned).to_vec(),
                    path_absolute: true,
                    query: "".to_owned(),
                    fragment: "Related".to_owned()
                },
                version: HttpVersion(1, 1)
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn parse_headers() -> Result<(), ParseError> {
        let mut reader = Cursor::new("header1: value1\r\nheader2:value2  \r\n");
        let headers = parse::parse_headers(&mut reader).await?;
        assert_eq!(
            headers,
            HashMap::from([
                ("header1".to_owned(), "value1".as_bytes().to_owned()),
                ("header2".to_owned(), "value2".as_bytes().to_owned()),
            ])
        );
        Ok(())
    }

    #[tokio::test]
    async fn parse_request_full() -> Result<(), ParseError> {
        let mut reader = Cursor::new(indoc! {"
            GET http://www.ics.uci.edu/pub/ietf/uri/#Related HTTP/1.1\r
            header1: value1\r
            header2:value2  \r
            Content-Length: 48\r
            \r
            <html><body>something</body></html>"});
        let start_line = parse_request(&mut reader).await?;
        assert_eq!(
            start_line,
            Request {
                start_line: StartLine {
                    method: "GET".to_owned(),
                    uri: Uri {
                        scheme: "http".to_owned(),
                        authority: "www.ics.uci.edu".to_owned(),
                        path: ["pub", "ietf", "uri"].map(&str::to_owned).to_vec(),
                        path_absolute: true,
                        query: "".to_owned(),
                        fragment: "Related".to_owned()
                    },
                    version: HttpVersion(1, 1)
                },
                headers: HashMap::from([
                    ("header1".to_owned(), "value1".as_bytes().to_owned()),
                    ("header2".to_owned(), "value2".as_bytes().to_owned()),
                    ("Content-Length".to_owned(), "48".as_bytes().to_owned())
                ]),
                body: "<html><body>something</body></html>".as_bytes().to_owned()
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn parse_request_no_body() -> Result<(), ParseError> {
        let mut reader = Cursor::new(indoc! {"
            GET http://www.ics.uci.edu/pub/ietf/uri/#Related HTTP/1.1\r
            header1: value1\r
            header2:value2  \r
            \r
        "});
        let start_line = parse::parse_request(&mut reader).await?;
        assert_eq!(
            start_line,
            Request {
                start_line: StartLine {
                    method: "GET".to_owned(),
                    uri: Uri {
                        scheme: "http".to_owned(),
                        authority: "www.ics.uci.edu".to_owned(),
                        path: ["pub", "ietf", "uri"].map(&str::to_owned).to_vec(),
                        path_absolute: true,
                        query: "".to_owned(),
                        fragment: "Related".to_owned()
                    },
                    version: HttpVersion(1, 1)
                },
                headers: HashMap::from([
                    ("header1".to_owned(), "value1".as_bytes().to_owned()),
                    ("header2".to_owned(), "value2".as_bytes().to_owned()),
                ]),
                body: "".as_bytes().to_owned()
            }
        );
        Ok(())
    }
}
