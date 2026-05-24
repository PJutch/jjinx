use std::{collections::HashMap, io};

use tokio::io::{AsyncWrite, AsyncWriteExt};

pub struct Response {
    pub status: i64,
    pub headers: HashMap<String, Vec<u8>>,
    pub body: Vec<u8>,
}

pub async fn write_response<Writer: AsyncWrite + Unpin>(
    writer: &mut Writer,
    response: &Response,
) -> Result<(), io::Error> {
    writer.write("HTTP/1.1 ".as_bytes()).await?;
    writer
        .write(itoa::Buffer::new().format(response.status).as_bytes())
        .await?;
    writer.write(" TODO\r\n".as_bytes()).await?;

    for (field_name, field_value) in &response.headers {
        writer.write(field_name.as_bytes()).await?;
        writer.write(": ".as_bytes()).await?;
        writer.write(&field_value).await?;
        writer.write("\r\n".as_bytes()).await?;
    }

    writer.write("\r\n".as_bytes()).await?;
    writer.write(&response.body).await?;

    writer.flush().await?;
    Ok(())
}
