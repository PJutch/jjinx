use std::net::IpAddr;
use std::time::Duration;

use tokio::io::AsyncBufRead;
use tokio::time::timeout;

use super::io::{read_byte, read_token, try_skip_newline};
use super::parse::{parse_body, parse_headers, parse_http_version};
use super::{ParseError, Request, StartLine};
use crate::http::io::read_until;
use crate::uri::parse_uri;

async fn parse_start_line<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
) -> Result<StartLine, ParseError> {
    let method = read_token(reader).await?;
    if read_byte(reader).await? != b' ' {
        return Err(ParseError::ExpectedSpace);
    }

    let mut uri = Vec::new();
    read_until(reader, |c| c == b' ', &mut uri).await?;
    let uri = String::from_utf8(uri).map_err(ParseError::Utf8Error)?;

    let uri = parse_uri(&uri).map_err(ParseError::UriError)?;
    if read_byte(reader).await? != b' ' {
        return Err(ParseError::ExpectedSpace);
    }

    let version = parse_http_version(reader).await?;

    Ok(StartLine {
        method,
        uri,
        version,
    })
}

pub async fn parse_request<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
    ip: IpAddr,
    header_timeout: Duration,
    body_timeout: Duration,
) -> Result<Request, ParseError> {
    let (start_line, mut headers) = timeout(header_timeout, async {
        let start_line = parse_start_line(reader).await?;
        try_skip_newline(reader).await?;
        let headers = parse_headers(reader).await?;
        try_skip_newline(reader).await?;
        Ok((start_line, headers))
    })
    .await
    .map_err(|_| ParseError::Timeout)??;

    let body = parse_body(reader, &mut headers, body_timeout).await?;

    Ok(Request {
        ip,
        start_line,
        headers,
        body,
    })
}

#[cfg(test)]
mod tests {
    use crate::http::{
        HttpVersion, ParseError, Request, StartLine, parse, parse_request, request_parse,
    };
    use crate::uri::parse_uri;
    use indoc::indoc;
    use std::time::Duration;
    use std::{collections::HashMap, io::Cursor};

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
        let start_line = request_parse::parse_start_line(&mut reader).await?;
        assert_eq!(
            start_line,
            StartLine {
                method: "GET".to_owned(),
                uri: parse_uri("http://www.ics.uci.edu/pub/ietf/uri/#Related").unwrap(),
                version: HttpVersion(1, 1)
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn parse_headers() -> Result<(), ParseError> {
        let mut reader = Cursor::new("Header1: value1\r\nheader2:value2  \r\n");
        let headers = parse::parse_headers(&mut reader).await?;
        assert_eq!(
            headers,
            HashMap::from([
                ("Header1".to_owned(), "value1".to_owned()),
                ("Header2".to_owned(), "value2".to_owned()),
            ])
        );
        Ok(())
    }

    #[tokio::test]
    async fn parse_request_full() -> Result<(), ParseError> {
        let mut reader = Cursor::new(indoc! {"
            GET http://www.ics.uci.edu/pub/ietf/uri/#Related HTTP/1.1\r
            Header1: value1\r
            header2:value2  \r
            Content-Length: 48\r
            \r
            <html><body>something</body></html>"});
        let ip = "127.0.0.1".parse().unwrap();

        let request = parse_request(&mut reader, ip, Duration::MAX, Duration::MAX).await?;

        assert_eq!(
            request,
            Request {
                ip,
                start_line: StartLine {
                    method: "GET".to_owned(),
                    uri: parse_uri("http://www.ics.uci.edu/pub/ietf/uri/#Related").unwrap(),
                    version: HttpVersion(1, 1)
                },
                headers: HashMap::from([
                    ("Header1".to_owned(), "value1".to_owned()),
                    ("Header2".to_owned(), "value2".to_owned()),
                    ("Content-Length".to_owned(), "48".to_owned())
                ]),
                body: b"<html><body>something</body></html>".to_vec()
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn parse_request_no_body() -> Result<(), ParseError> {
        let mut reader = Cursor::new(indoc! {"
            GET http://www.ics.uci.edu/pub/ietf/uri/#Related HTTP/1.1\r
            Header1: value1\r
            header2:value2  \r
            \r
        "});
        let ip = "127.0.0.1".parse().unwrap();

        let request = parse_request(&mut reader, ip, Duration::MAX, Duration::MAX).await?;

        assert_eq!(
            request,
            Request {
                ip,
                start_line: StartLine {
                    method: "GET".to_owned(),
                    uri: parse_uri("http://www.ics.uci.edu/pub/ietf/uri/#Related").unwrap(),
                    version: HttpVersion(1, 1)
                },
                headers: HashMap::from([
                    ("Header1".to_owned(), "value1".to_owned()),
                    ("Header2".to_owned(), "value2".to_owned()),
                ]),
                body: b"".to_vec()
            }
        );
        Ok(())
    }
}
