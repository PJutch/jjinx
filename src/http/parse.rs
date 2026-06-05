use super::io::{
    is_whitespace, peek_byte, read_byte, read_digit, read_n_bytes, read_token, read_until,
    skip_fixed, skip_whitespace, try_skip_newline,
};
use super::{HttpVersion, ParseError};
use std::collections::HashMap;
use tokio::io::AsyncBufRead;

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

pub async fn parse_body<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
    headers: &HashMap<String, String>,
) -> Result<Vec<u8>, ParseError> {
    Ok(if let Some(data) = headers.get("Content-Length") {
        let len = data.parse().map_err(ParseError::ParseIntError)?;
        read_n_bytes(reader, len).await?
    } else if let Some(_) = headers.get("Transfer-Encoding") {
        todo!("Handle Transfer-Encoding");
    } else {
        Vec::new()
    })
}
