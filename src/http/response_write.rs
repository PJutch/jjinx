use super::{Response, SendError};

use std::time::Duration;

use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    time::timeout,
};

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
        writer.write(b" TODO\r\n").await?;

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

        writer.write(b"\r\n").await?;
        writer.write(&response.body).await?;

        writer.flush().await?;
        Ok(())
    })
    .await
    .map_err(|_| SendError::Timeout)?
    .map_err(SendError::IoError)
}
