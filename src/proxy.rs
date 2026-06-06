use std::collections::HashMap;

use futures::io;
use thiserror::Error;
use tokio::io::{BufReader, BufWriter};
use tokio::net::TcpStream;

use crate::http::{self, HttpVersion, Request, Response, parse_response, write_request};
use crate::uri::Uri;

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("Io error: {0}")]
    IoError(io::Error),
    #[error("Response parse error: {0}")]
    ResponseParseError(http::ParseError),
}

pub async fn proxy_pass(
    uri: Uri,
    request: &Request,
    matching_prefix_len: usize,
    new_headers: HashMap<String, String>,
) -> Result<Response, ProxyError> {
    let mut proxy_request = request.clone();
    proxy_request.start_line.version = HttpVersion(1, 1);

    if !proxy_request.start_line.uri.host().is_empty() {
        proxy_request.headers.insert(
            "Host".to_string(),
            proxy_request.start_line.uri.host().to_owned(),
        );
    }

    if !uri.path().is_empty() {
        proxy_request
            .start_line
            .uri
            .replace_path_prefix(matching_prefix_len, uri.path());
    }

    for (header_name, header_value) in new_headers {
        proxy_request.headers.insert(header_name, header_value);
    }

    let mut stream = TcpStream::connect(uri.host())
        .await
        .map_err(ProxyError::IoError)?;

    write_request(&mut BufWriter::new(&mut stream), &proxy_request)
        .await
        .map_err(ProxyError::IoError)?;

    Ok(parse_response(&mut BufReader::new(&mut stream))
        .await
        .map_err(ProxyError::ResponseParseError)?)
}
