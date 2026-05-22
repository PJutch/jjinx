use tokio::io::BufReader;
use tokio::net::TcpListener;

mod parse;
use parse::parse_request;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:8080").await?;
    loop {
        let (socket, _addr) = listener.accept().await?;
        tokio::spawn(async move {
            let mut reader = BufReader::new(socket);
            match parse_request(&mut reader).await {
                Ok(()) => {}
                Err(err) => println!("{err}"),
            }
        });
    }
}
