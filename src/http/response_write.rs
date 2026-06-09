use super::{Response, SendError};

use std::time::Duration;

use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    time::timeout,
};

fn status_name(status: i16) -> &'static str {
    match status {
        100 => "Continue",
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        203 => "Non-Authoritative Information",
        204 => "No Content",
        205 => "Reset Content",
        206 => "Partial Content",
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        305 => "Use Proxy",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        407 => "Proxy Authentication Required",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        412 => "Precondition Failed",
        413 => "Content Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        416 => "Range Not Satisfiable",
        417 => "Expectation Failed",
        421 => "Misdirected Request",
        422 => "Unprocessable Content",
        426 => "Upgrade Required",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        _ => "Unknown",
    }
}

pub async fn write_response<Writer: AsyncWrite + Unpin>(
    writer: &mut Writer,
    response: &Response,
    send_timeout: Duration,
) -> Result<(), SendError> {
    timeout(send_timeout, async {
        writer.write(b"HTTP/1.1 ").await?;
        writer
            .write(itoa::Buffer::new().format(response.status).as_bytes())
            .await?;
        writer.write(b" ").await?;
        writer
            .write(status_name(response.status).as_bytes())
            .await?;
        writer.write("\r\n".as_bytes()).await?;

        for (field_name, field_value) in &response.headers {
            writer.write(field_name.as_bytes()).await?;
            writer.write(b": ").await?;
            writer.write(field_value.as_bytes()).await?;
            writer.write(b"\r\n").await?;
        }

        if !response.headers.contains_key("Content-Length") {
            writer.write(b"Content-Length: ").await?;
            writer
                .write(itoa::Buffer::new().format(response.body.len()).as_bytes())
                .await?;
            writer.write(b"\r\n").await?;
        }

        if !response.headers.contains_key("Connection") {
            writer.write(b"Connection: close\r\n").await?;
        }

        writer.write(b"\r\n").await?;

        if response.body.is_empty() {
            writer.flush().await?;
        }

        for chunk in response.body.chunks(1024) {
            writer.write(chunk).await?;
            writer.flush().await?;
        }

        Ok(())
    })
    .await
    .map_err(|_| SendError::Timeout)?
    .map_err(SendError::IoError)
}
