use crate::http::io::{read_n_bytes_to, skip_until_inclusive};

use super::io::{
    is_whitespace, peek_byte, read_byte, read_digit, read_n_bytes, read_token, read_until,
    skip_fixed, skip_whitespace, try_skip_newline,
};
use super::{HttpVersion, ParseError};

use std::collections::HashMap;
use std::time::Duration;
use tokio::io::AsyncBufRead;
use tokio::time::timeout;

pub async fn parse_http_version<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
) -> Result<HttpVersion, ParseError> {
    skip_fixed(reader, "HTTP/").await.and_then(|matched| {
        if matched {
            Ok(())
        } else {
            Err(ParseError::ParseVersionError)
        }
    })?;

    let digit1 = read_digit(reader).await?;

    if read_byte(reader).await? != b'.' {
        return Err(ParseError::ParseVersionError);
    }

    let digit2 = read_digit(reader).await?;

    Ok(HttpVersion(digit1, digit2))
}

fn to_pascal_case(str: &str) -> String {
    let mut result = String::new();
    let mut prev_letter = false;
    for c in str.chars() {
        if !prev_letter {
            result.push(c.to_ascii_uppercase());
        } else {
            result.push(c.to_ascii_lowercase());
        }

        prev_letter = c.is_ascii_alphabetic();
    }
    result
}

pub async fn parse_headers<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
) -> Result<HashMap<String, String>, ParseError> {
    let mut headers = HashMap::new();
    loop {
        let first_byte = peek_byte(reader).await?;
        if first_byte == Some(b'\r') || first_byte == None {
            return Ok(headers);
        }

        let field_name = read_token(reader).await?;
        if read_byte(reader).await? != b':' {
            return Err(ParseError::ExpectedColon);
        }

        skip_whitespace(reader).await?;

        let mut field_value = Vec::new();
        read_until(reader, |c| c == b'\r', &mut field_value).await?;
        while field_value.last().copied().is_some_and(is_whitespace) {
            field_value.pop();
        }

        headers.insert(
            to_pascal_case(&field_name),
            String::from_utf8(field_value).map_err(ParseError::Utf8Error)?,
        );
        try_skip_newline(reader).await?;
    }
}

async fn parse_chunked_body<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
    per_read_timeout: Duration,
) -> Result<Vec<u8>, ParseError> {
    let mut result = Vec::new();
    loop {
        let mut length = Vec::new();

        timeout(per_read_timeout, async {
            read_until(reader, |c| c == b' ' || c == b'\r', &mut length).await?;
            skip_until_inclusive(reader, |c| c == b'\n').await?;
            Ok::<_, ParseError>(())
        })
        .await
        .map_err(|_| ParseError::Timeout)??;

        if length.is_empty() {
            break;
        }

        let length = str::from_utf8(&length).map_err(ParseError::Utf8ErrorStr)?;
        let length = usize::from_str_radix(length, 16).map_err(ParseError::ParseIntError)?;
        if length == 0 {
            break;
        }

        read_n_bytes_to(reader, length, per_read_timeout, &mut result).await?;
    }

    let mut skip_trailing = true;
    while  skip_trailing {
        timeout(per_read_timeout, async {
            if peek_byte(reader).await?.is_none_or(|c| c == b'\r') {
                skip_trailing = false;
            }

            skip_until_inclusive(reader, |c| c == b'\n').await?;
            Ok(())
        })
        .await
        .map_err(|_| ParseError::Timeout)??;
    }

    Ok(result)
}

pub async fn parse_body<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
    headers: &mut HashMap<String, String>,
    timeout: Duration,
) -> Result<Vec<u8>, ParseError> {
    Ok(
        if let Some(transfer_encoding) = headers.remove("Transfer-Encoding") {
            if transfer_encoding == "chunked" {
                parse_chunked_body(reader, timeout).await?
            } else {
                return Err(ParseError::UnknownTransferEncoding(
                    transfer_encoding.clone(),
                ));
            }
        } else if let Some(data) = headers.get("Content-Length") {
            let len = data.parse().map_err(ParseError::ParseIntError)?;
            read_n_bytes(reader, len, timeout).await?
        } else {
            Vec::new()
        },
    )
}
