use std::time::Duration;

use tokio::io::AsyncBufRead;
use tokio::time::timeout;

use super::io::{read_byte, read_digit, skip_fixed, skip_until_inclusive};
use super::parse::{parse_body, parse_headers, parse_http_version};
use super::{ParseError, Response};

pub async fn parse_response<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
    header_timeout: Duration,
    body_timeout: Duration,
) -> Result<Response, ParseError> {
    let (status, mut headers) = timeout(header_timeout, async {
        let http_version = parse_http_version(reader).await?;
        if http_version.0 != 1 {
            return Err(ParseError::InvalidHttpVersion(http_version));
        }

        if read_byte(reader).await? != b' ' {
            return Err(ParseError::ExpectedSpace);
        }

        let mut status = 0;
        for _ in 0..3 {
            status *= 10;
            status += read_digit(reader).await? as i16;
        }

        skip_until_inclusive(reader, |c| c == b'\n').await?;

        let headers = parse_headers(reader).await?;

        skip_fixed(reader, "\r\n").await?;
        Ok((status, headers))
    })
    .await
    .map_err(|_| ParseError::Timeout)??;

    let body = parse_body(reader, &mut headers, body_timeout).await?;

    Ok(Response {
        status,
        headers,
        body,
    })
}
