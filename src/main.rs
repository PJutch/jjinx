use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};

use tokio::io::{BufReader, BufWriter};
use tokio::net::TcpListener;

mod parse;
use parse::parse_request;

mod respond;
use respond::{Response, write_response};

mod config;

fn read_file(path: &str) -> Result<Vec<u8>, io::Error> {
    let mut result = Vec::new();
    File::open(path)?.read_to_end(&mut result)?;
    return Ok(result);
}

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

            let response = match read_file("index.html") {
                Ok(data) => data,
                Err(err) => {
                    println!("{err}");
                    return;
                }
            };

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
                    body: response,
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
