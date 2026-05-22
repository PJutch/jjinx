use std::io;
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Unexpected eof")]
    UnexpectedEof,
    #[error("Io error: {0}")]
    IoError(io::Error),
    #[error("Expected a digit")]
    ExpectedDigit,
    #[error("Invalid HTTP version")]
    InvalidVersion,
    #[error("Expected space")]
    ExpectedSpace,
    #[error("Uri parse error")]
    UriParseError,
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

async fn read_fixed<Reader: AsyncBufRead + Unpin>(
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
    read_fixed(reader, "HTTP/").await.and_then(|matched| {
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

// expects reader to be buffered
pub async fn parse_request<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
) -> Result<(), ParseError> {
    parse_start_line(reader).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::parse::{self, HttpVersion, ParseError, StartLine, Uri, parse_uri};
    use std::io::Cursor;

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
}
