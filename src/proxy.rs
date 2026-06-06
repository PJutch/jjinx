use std::collections::HashMap;

use futures::io;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::io::{BufReader, BufWriter};

use crate::http::{self, HttpVersion, Request, Response, write_request, parse_response};
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
