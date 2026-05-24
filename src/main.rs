use std::collections::HashMap;

use tokio::io::{BufReader, BufWriter};
use tokio::net::TcpListener;

mod parse;
use parse::parse_request;

mod respond;
use respond::{Response, write_response};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:8080").await?;
    loop {
        let (mut socket, _addr) = listener.accept().await?;
        tokio::spawn(async move {
            let mut reader = BufReader::new(&mut socket);
            let request = match parse_request(&mut reader).await {
                Ok(request) => request,
                Err(err) => {
                    println!("{err}");
                    return;
                }
            };

            println!("{request:?}");

            let response = "<html><head><title>test</title></head><body><p>Hello!</p></body></html>\n";
            let mut writer = BufWriter::new(&mut socket);
            if let Err(err) = write_response(
                &mut writer,
                &Response {
                    status: 200,
                    headers: HashMap::from([(
                        "Content-Length".to_string(),
                        itoa::Buffer::new()
                            .format(response.len())
                            .as_bytes()
                            .to_vec(),
                    )]),
                    body: response.as_bytes().to_vec()
                },
            )
            .await
            {
                println!("{err}");
                return;
            }
        });
    }
}
