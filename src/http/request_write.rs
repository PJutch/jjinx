use std::io;
use tokio::io::{AsyncWrite, AsyncWriteExt};

use super::Request;

pub async fn write_request<Writer: AsyncWrite + Unpin>(
    writer: &mut Writer,
    request: &Request,
) -> Result<(), io::Error> {
    writer.write(request.start_line.method.as_bytes()).await?;

    writer.write(b" ").await?;
    writer.write(request.start_line.uri.full.as_bytes()).await?;

    writer.write(b" HTTP").await?;
    writer.write(b"/").await?;
    writer.write(&[request.start_line.version.0 + b'0']).await?;
    writer.write(b".").await?;
    writer.write(&[request.start_line.version.0 + b'0']).await?;
    writer.write(b"\r\n").await?;

    for (name, value) in &request.headers {
        writer.write(name.as_bytes()).await?;
        writer.write(b": ").await?;
        writer.write(value.as_bytes()).await?;
        writer.write(b"\r\n").await?;
    }

    writer.write(b"\r\n").await?;
    writer.write(&request.body).await?;

    writer.flush().await?;

    Ok(())
}
