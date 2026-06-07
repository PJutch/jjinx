use std::time::Duration;

use super::ParseError;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};
use tokio::time::timeout;

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

pub async fn read_byte<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
) -> Result<u8, ParseError> {
    try_read_byte(reader)
        .await?
        .ok_or(ParseError::UnexpectedEof)
}

pub async fn peek_byte<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
) -> Result<Option<u8>, ParseError> {
    let buf = reader.fill_buf().await.map_err(ParseError::IoError)?;
    Ok(buf.first().copied())
}

pub async fn read_digit<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
) -> Result<u8, ParseError> {
    let c = read_byte(reader).await?;
    if c.is_ascii_digit() {
        Ok(c - b'0')
    } else {
        Err(ParseError::ExpectedDigit)
    }
}

pub async fn skip_fixed<Reader: AsyncBufRead + Unpin>(
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

fn is_tchar(c: u8) -> bool {
    c.is_ascii_alphanumeric()
        || [
            '!', '#', '$', '%', '&', '\'', '*', '+', '-', '.', '^', '_', '`', '|', '~',
        ]
        .map(|c| c as u8)
        .contains(&c)
}

pub async fn read_token<Reader: AsyncBufRead + Unpin>(
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

pub async fn read_until<Reader: AsyncBufRead + Unpin, F: Fn(u8) -> bool>(
    reader: &mut Reader,
    until: F,
    output: &mut Vec<u8>,
) -> Result<(), ParseError> {
    loop {
        if let Some(c) = peek_byte(reader).await? {
            if until(c) {
                break;
            }

            output.push(c);
            reader.consume(1);
        } else {
            break;
        }
    }
    Ok(())
}

pub async fn skip_until_inclusive<Reader: AsyncBufRead + Unpin, F: Fn(u8) -> bool>(
    reader: &mut Reader,
    until: F,
) -> Result<(), ParseError> {
    loop {
        if let Some(c) = peek_byte(reader).await? {
            reader.consume(1);
            if until(c) {
                break;
            }
        } else {
            break;
        }
    }
    Ok(())
}

pub async fn try_skip_newline<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
) -> Result<bool, ParseError> {
    match peek_byte(reader).await? {
        Some(0xD) => {
            reader.consume(1);
            if read_byte(reader).await? != b'\n' {
                return Err(ParseError::ExpectedLineFeed);
            }
            Ok(true)
        }
        Some(_) => Err(ParseError::ExpectedNewline),
        None => Ok(false),
    }
}

pub fn is_whitespace(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

pub async fn skip_whitespace<Reader: AsyncBufRead + Unpin>(
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

pub async fn read_n_bytes<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
    n: usize,
    per_read_timeout: Duration,
) -> Result<Vec<u8>, ParseError> {
    let mut result = Vec::new();
    while result.len() < n {
        let mut buf = timeout(per_read_timeout, reader.fill_buf())
            .await
            .map_err(|_| ParseError::Timeout)?
            .map_err(ParseError::IoError)?;

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
