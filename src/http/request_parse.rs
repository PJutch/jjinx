use tokio::io::AsyncBufRead;

use super::io::{read_byte, read_token, try_skip_newline};
use super::parse::{parse_body, parse_headers, parse_http_version};
use super::uri::parse_uri;
use super::{ParseError, Request, StartLine};

async fn parse_start_line<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
) -> Result<StartLine, ParseError> {
    let method = read_token(reader).await?;
    if read_byte(reader).await? != b' ' {
        return Err(ParseError::ExpectedSpace);
    }

    let uri = parse_uri(reader).await?;
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
) -> Result<Request, ParseError> {
    let start_line = parse_start_line(reader).await?;
    try_skip_newline(reader).await?;
    let headers = parse_headers(reader).await?;
    try_skip_newline(reader).await?;

    let body = parse_body(reader, &headers).await?;

    Ok(Request {
        start_line,
        headers,
        body,
    })
}

#[cfg(test)]
mod tests {
    use crate::http::{
        HttpVersion, ParseError, Request, StartLine, Uri, parse, parse_request, request_parse,
        uri::parse_uri,
    };
    use indoc::indoc;
    use std::{collections::HashMap, io::Cursor};

    #[tokio::test]
    async fn parse_uri_full() -> Result<(), ParseError> {
        let mut reader = Cursor::new("http://www.ics.uci.edu/pub/ietf/uri/#Related");

        let expected = Uri {
            full: "http://www.ics.uci.edu/pub/ietf/uri/#Related".to_owned(),
            host: "www.ics.uci.edu".to_owned(),
            path: "/pub/ietf/uri/".to_owned(),
        };

        let uri = parse_uri(&mut reader).await?;
        assert_eq!(uri, expected);
        Ok(())
    }

    #[tokio::test]
    async fn parse_uri_no_authority() -> Result<(), ParseError> {
        let mut reader = Cursor::new("/pub/ietf/uri/#Related");

        let expected = Uri {
            full: "/pub/ietf/uri/#Related".to_owned(),
            host: "".to_owned(),
            path: "/pub/ietf/uri/".to_owned(),
        };

        let uri = parse_uri(&mut reader).await?;
        assert_eq!(uri, expected);
        Ok(())
    }

    #[tokio::test]
    async fn parse_uri_percents() -> Result<(), ParseError> {
        let mut reader = Cursor::new("/%70ub/ietf/uri/#Related");

        let expected = Uri {
            full: "/%70ub/ietf/uri/#Related".to_owned(),
            host: "".to_owned(),
            path: "/pub/ietf/uri/".to_owned(),
        };

        let uri = parse_uri(&mut reader).await?;
        assert_eq!(uri, expected);
        Ok(())
    }

    #[tokio::test]
    async fn parse_uri_path_compression() -> Result<(), ParseError> {
        let mut reader = Cursor::new("/..//./stays/removed/../#Related");

        let expected = Uri {
            full: "/..//./stays/removed/../#Related".to_owned(),
            host: "".to_owned(),
            path: "/../stays/".to_owned(),
        };

        let uri = parse_uri(&mut reader).await?;
        assert_eq!(uri, expected);
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
        let start_line = request_parse::parse_start_line(&mut reader).await?;
        assert_eq!(
            start_line,
            StartLine {
                method: "GET".to_owned(),
                uri: Uri {
                    full: "http://www.ics.uci.edu/pub/ietf/uri/#Related".to_owned(),
                    host: "www.ics.uci.edu".to_owned(),
                    path: "/pub/ietf/uri/".to_owned(),
                },
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
        let start_line = parse_request(&mut reader).await?;
        assert_eq!(
            start_line,
            Request {
                start_line: StartLine {
                    method: "GET".to_owned(),
                    uri: Uri {
                        full: "http://www.ics.uci.edu/pub/ietf/uri/#Related".to_owned(),
                        host: "www.ics.uci.edu".to_owned(),
                        path: "/pub/ietf/uri/".to_owned(),
                    },
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
        let start_line = parse_request(&mut reader).await?;
        assert_eq!(
            start_line,
            Request {
                start_line: StartLine {
                    method: "GET".to_owned(),
                    uri: Uri {
                        full: "http://www.ics.uci.edu/pub/ietf/uri/#Related".to_owned(),
                        host: "www.ics.uci.edu".to_owned(),
                        path: "/pub/ietf/uri/".to_owned(),
                    },
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
