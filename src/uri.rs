use std::num::ParseIntError;

use thiserror::Error;

#[derive(Debug, Default, PartialEq, Clone)]
pub struct Uri {
    pub full: String,
    scheme_end: usize,
    domain: (usize, usize),
    pub port: Option<u16>,
    host_end: usize,
    path: (usize, usize),
    query: (usize, usize),
    fragment_start: usize,
}

impl Uri {
    pub fn scheme(&self) -> &str {
        &self.full[..self.scheme_end]
    }

    pub fn domain(&self) -> &str {
        &self.full[self.domain.0..self.domain.1]
    }

    pub fn host(&self) -> &str {
        &self.full[self.domain.0..self.host_end]
    }

    pub fn path(&self) -> &str {
        &self.full[self.path.0..self.path.1]
    }

    pub fn query(&self) -> &str {
        &self.full[self.query.0..self.query.1]
    }

    pub fn fragment(&self) -> &str {
        &self.full[self.fragment_start..]
    }
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("Invalid percent encoding: unexpected EOF")]
    CutPersonEncoding,
    #[error("Invalid percent encoding: {0} isn't a hex digit")]
    InvalidParcentEncodingDigit(char),
    #[error("Error parsing port")]
    PortError(ParseIntError),
}

fn compress_last_component(path: &mut String, last_slash: &mut usize) {
    if &path[*last_slash..] == "/." || path == "." {
        path.pop();
        path.pop();
    } else if &path[*last_slash..] == "/.." {
        for _ in 0..3 {
            path.pop();
        }

        *last_slash = path.bytes().rposition(|c| c == b'/').unwrap_or(0);

        if &path[*last_slash..] != "/.." && path != ".." && !path.is_empty() {
            while path.len() > *last_slash {
                path.pop();
            }
        } else {
            for c in "/..".chars() {
                path.push(c);
            }
        }
    }
}

fn uri_decode(data: &str, output: &mut String) -> Result<(), ParseError> {
    let mut last_slash = 0;
    let mut percent_encoded_digits = 0;
    let mut perecent_encoded_byte = 0;

    for c in data.chars() {
        if percent_encoded_digits > 0 {
            perecent_encoded_byte <<= 4;
            perecent_encoded_byte +=
                c.to_digit(16)
                    .ok_or(ParseError::InvalidParcentEncodingDigit(c))? as u8;

            percent_encoded_digits -= 1;
            if percent_encoded_digits == 0 {
                if percent_encoded_digits != b'/' {
                    output.push(perecent_encoded_byte as char);
                    perecent_encoded_byte = 0;
                }
            }
        } else if c == '%' {
            percent_encoded_digits = 2;
        } else if c == '/' {
            if last_slash + 1 == output.len() {
                continue;
            } else {
                compress_last_component(output, &mut last_slash);
                output.push(c);
                last_slash = output.len() - 1;
            }
        } else {
            output.push(c);
        }
    }

    compress_last_component(output, &mut last_slash);

    if percent_encoded_digits > 0 {
        return Err(ParseError::CutPersonEncoding);
    }

    Ok(())
}

fn split_domain_port(uri: &mut Uri) -> Result<(), ParseError> {
    if let Some(domain_end) = uri.host().find(':') {
        uri.domain.1 = uri.domain.0 + domain_end;
        uri.port = Some(
            uri.host()[domain_end + 1..]
                .parse()
                .map_err(ParseError::PortError)?,
        );
    } else {
        uri.domain.1 = uri.host_end;
        uri.port = None;
    }
    Ok(())
}

pub fn parse_uri(mut str: &str) -> Result<Uri, ParseError> {
    let mut uri = Uri::default();

    if !str.starts_with('/') {
        if let Some(scheme_end) = str.find(':') {
            uri.scheme_end = scheme_end;
            uri.full.push_str(&str[..scheme_end + 1]);

            str = &str[scheme_end + 1..];
        } else {
            uri.full = str.to_owned();
            uri.host_end = uri.full.len();
            split_domain_port(&mut uri)?;
            return Ok(uri);
        }
    }

    if str.starts_with("//") || !str.starts_with('/') && uri.scheme().is_empty() {
        if let Some(new_str) = str.strip_prefix("//") {
            uri.full.push_str("//");
            str = new_str;
        }

        uri.domain.0 = uri.full.len();

        let host_end = str.find('/').unwrap_or(str.len());

        uri.full.push_str(&str[..host_end]);
        uri.host_end = uri.full.len();
        split_domain_port(&mut uri)?;

        str = &str[host_end..];
    }

    let path_end = str.find(|c| c == '?' || c == '#').unwrap_or(str.len());

    uri.path.0 = uri.full.len();
    uri_decode(&str[..path_end], &mut uri.full)?;
    uri.path.1 = uri.full.len();

    str = &str[path_end..];

    if let Some(new_str) = str.strip_prefix('?') {
        uri.full.push('?');

        uri.query.0 = uri.full.len();

        let query_end = new_str.find('#').unwrap_or(str.len());
        uri.full.push_str(&new_str[..query_end]);

        uri.query.1 = uri.full.len();

        str = &new_str[query_end..];
    }

    if str.starts_with('#') {
        uri.fragment_start = uri.full.len() + 1;
        uri.full.push_str(str);
    }

    Ok(uri)
}

#[cfg(test)]
mod tests {
    use super::{ParseError, parse_uri};

    #[tokio::test]
    async fn parse_uri_full() -> Result<(), ParseError> {
        let string = "http://www.ics.uci.edu:80/pub/ietf/uri/#Related";
        let uri = parse_uri(&string)?;

        assert_eq!(uri.full, string);
        assert_eq!(uri.scheme(), "http");
        assert_eq!(uri.domain(), "www.ics.uci.edu");
        assert_eq!(uri.port, Some(80));
        assert_eq!(uri.host(), "www.ics.uci.edu:80");
        assert_eq!(uri.path(), "/pub/ietf/uri/");
        assert_eq!(uri.query(), "");
        assert_eq!(uri.fragment(), "Related");

        Ok(())
    }

//     #[tokio::test]
//     async fn parse_uri_no_authority() -> Result<(), ParseError> {
//         let mut reader = Cursor::new("/pub/ietf/uri/#Related");

//         let expected = Uri {
//             full: "/pub/ietf/uri/#Related".to_owned(),
//             host: "".to_owned(),
//             path: "/pub/ietf/uri/".to_owned(),
//         };

//         let uri = parse_uri(&mut reader).await?;
//         assert_eq!(uri, expected);
//         Ok(())
//     }

//     #[tokio::test]
//     async fn parse_uri_percents() -> Result<(), ParseError> {
//         let mut reader = Cursor::new("/%70ub/ietf/uri/#Related");

//         let expected = Uri {
//             full: "/%70ub/ietf/uri/#Related".to_owned(),
//             host: "".to_owned(),
//             path: "/pub/ietf/uri/".to_owned(),
//         };

//         let uri = parse_uri(&mut reader).await?;
//         assert_eq!(uri, expected);
//         Ok(())
//     }

//     #[tokio::test]
//     async fn parse_uri_path_compression() -> Result<(), ParseError> {
//         let mut reader = Cursor::new("/..//./stays/removed/../#Related");

//         let expected = Uri {
//             full: "/..//./stays/removed/../#Related".to_owned(),
//             host: "".to_owned(),
//             path: "/../stays/".to_owned(),
//         };

//         let uri = parse_uri(&mut reader).await?;
//         assert_eq!(uri, expected);
//         Ok(())
//     }
}
