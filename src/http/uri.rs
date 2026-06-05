use super::io::{peek_byte, read_until, read_until_inclusive};
use super::{ParseError, Uri};
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

fn is_uri_path_char(c: u8) -> bool {
    c.is_ascii_alphanumeric()
        || [
            '-', '.', '_', '~', ',', '[', ']', '!', '%', '$', '&', ':', '@', '\'', '(', ')', '*',
            '+', ',', ';', '=', '/',
        ]
        .map(|c| c as u8)
        .contains(&c)
}

fn hex_digit_to_i8(digit: u8) -> Result<u8, ParseError> {
    if digit.is_ascii_digit() {
        Ok(digit - b'0')
    } else if b'a' <= digit && digit <= b'f' {
        Ok(digit - b'a' + 10)
    } else if b'A' <= digit && digit <= b'F' {
        Ok(digit - b'A' + 10)
    } else {
        Err(ParseError::InvalidParcentEncodingDigit(digit))
    }
}

fn compress_last_component(path: &mut Vec<u8>, last_slash: &mut usize) {
    if &path[*last_slash..] == b"/." || path == b"." {
        path.pop();
        path.pop();
    } else if &path[*last_slash..] == b"/.." {
        for _ in 0..3 {
            path.pop();
        }

        *last_slash = path.iter().copied().rposition(|c| c == b'/').unwrap_or(0);

        if &path[*last_slash..] != b"/.." && path != b".." && !path.is_empty() {
            while path.len() > *last_slash {
                path.pop();
            }
        } else {
            for c in "/..".bytes() {
                path.push(c);
            }
        }
    }
}

fn uri_decode(data: &[u8]) -> Result<String, ParseError> {
    let mut decoded = Vec::new();
    let mut last_slash = 0;
    let mut percent_encoded_digits = 0;
    let mut perecent_encoded_byte = 0;

    for c in data.iter().copied() {
        if percent_encoded_digits > 0 {
            perecent_encoded_byte <<= 4;
            perecent_encoded_byte += hex_digit_to_i8(c)?;

            percent_encoded_digits -= 1;
            if percent_encoded_digits == 0 {
                if percent_encoded_digits != b'/' {
                    decoded.push(perecent_encoded_byte);
                    perecent_encoded_byte = 0;
                }
            }
        } else if c == b'%' {
            percent_encoded_digits = 2;
        } else if c == b'/' {
            if last_slash + 1 == decoded.len() {
                continue;
            } else {
                compress_last_component(&mut decoded, &mut last_slash);
                decoded.push(c);
                last_slash = decoded.len() - 1;
            }
        } else {
            decoded.push(c);
        }
    }

    compress_last_component(&mut decoded, &mut last_slash);

    if percent_encoded_digits > 0 {
        return Err(ParseError::CutPersonEncoding);
    }

    Ok(String::from_utf8(decoded).map_err(ParseError::Utf8Error)?)
}

pub async fn parse_uri<Reader: AsyncBufRead + Unpin>(reader: &mut Reader) -> Result<Uri, ParseError> {
    let mut uri = Vec::new();
    let mut path_start = 0;
    let mut host = String::new();

    if peek_byte(reader).await? != Some(b'/') {
        read_until_inclusive(reader, |c| c == b':', &mut uri).await?;

        path_start = uri.len();
        if peek_byte(reader).await? == Some(b'/') {
            uri.push(b'/');
            reader.consume(1);

            if peek_byte(reader).await? == Some(b'/') {
                uri.push(b'/');
                reader.consume(1);

                let host_start = uri.len();
                read_until(reader, |c| c == b'/' || c == b':', &mut uri).await?;
                host = String::from_utf8(uri[host_start..].to_owned())
                    .map_err(ParseError::Utf8Error)?;

                if peek_byte(reader).await? == Some(b':') {
                    read_until(reader, |c| c == b'/', &mut uri).await?;
                }

                path_start = uri.len();
            }
        }
    }

    loop {
        if let Some(c) = peek_byte(reader).await? {
            if !is_uri_path_char(c) {
                break;
            }

            uri.push(c);
            reader.consume(1);
        } else {
            break;
        }
    }
    let path = uri_decode(&uri[path_start..])?;

    read_until(reader, |c| c == b' ', &mut uri).await?;
    let full = String::from_utf8(uri.clone()).map_err(ParseError::Utf8Error)?;

    Ok(Uri { full, path, host })
}
