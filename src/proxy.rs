use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::u32;

use futures::io;
use scopeguard::defer;
use thiserror::Error;
use tokio::io::{BufReader, BufWriter};
use tokio::net::TcpStream;
use twox_hash::XxHash64;

use crate::config;
use crate::http::{self, HttpVersion, Request, Response, parse_response, write_request};
use crate::uri::Uri;
use crate::vars::{VarError, replace_dynamic_vars};

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("Io error: {0}")]
    IoError(io::Error),
    #[error("Response parse error: {0}")]
    ResponseParseError(http::ParseError),
    #[error("Var error: {0}")]
    VarError(VarError),
}

enum Balancer {
    RoundRobin(AtomicUsize),
    LeastConnected(Vec<AtomicU32>),
    IpHash,
}

pub struct Upstream {
    balancer: Balancer,
    servers: Vec<String>,
}

impl Upstream {
    fn get_server(&self, ip: IpAddr) -> usize {
        match &self.balancer {
            Balancer::RoundRobin(next_server) => {
                let mut current_server = next_server.load(Ordering::Relaxed);
                loop {
                    match next_server.compare_exchange_weak(
                        current_server,
                        (current_server + 1) % self.servers.len(),
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => break current_server,
                        Err(new_current_server) => current_server = new_current_server,
                    }
                }
            }
            Balancer::LeastConnected(connection_counts) => {
                let mut best_server = 0;
                let mut best_count = u32::MAX;
                for (i, count) in connection_counts.iter().enumerate() {
                    let count = count.load(Ordering::Relaxed);
                    if count < best_count {
                        best_server = i;
                        best_count = count;
                    }
                }

                connection_counts[best_server].fetch_add(1, Ordering::Relaxed);

                best_server
            }
            Balancer::IpHash => {
                let mut hasher = XxHash64::default();
                ip.hash(&mut hasher);
                hasher.finish() as usize % self.servers.len()
            }
        }
    }

    fn close_connection(&self, server: usize) {
        if let Balancer::LeastConnected(connection_counts) = &self.balancer {
            connection_counts[server].fetch_sub(1, Ordering::Relaxed);
        }
    }
}

pub fn make_upstreams(config: HashMap<String, config::Upstream>) -> HashMap<String, Upstream> {
    config
        .into_iter()
        .map(|(name, config)| {
            (
                name,
                Upstream {
                    balancer: match config.balancing.unwrap_or(config::Balancing::RoundRobin) {
                        config::Balancing::RoundRobin => Balancer::RoundRobin(AtomicUsize::new(0)),
                        config::Balancing::LeastConnected => Balancer::LeastConnected(
                            vec![0u32; config.servers.len()]
                                .into_iter()
                                .map(AtomicU32::new)
                                .collect(),
                        ),
                        config::Balancing::IpHash => Balancer::IpHash,
                    },
                    servers: config.servers,
                },
            )
        })
        .collect()
}

pub async fn proxy_pass(
    uri: Uri,
    request: &Request,
    matching_prefix_len: usize,
    new_headers: HashMap<String, String>,
    upstreams: Arc<HashMap<String, Upstream>>,
) -> Result<Response, ProxyError> {
    let upstream = upstreams.get(uri.domain()).map(|upstream| {
        let server = upstream.get_server(request.ip);
        (upstream, server)
    });

    defer! {
        if let Some((upstream, server)) = upstream {
            upstream.close_connection(server);
        }
    }

    let host = if let Some((upstream, server)) = upstream {
        replace_dynamic_vars(&upstream.servers[server], request).map_err(ProxyError::VarError)?
    } else {
        String::new()
    };
    let host = if host.is_empty() { uri.host() } else { &host };

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

    let mut stream = TcpStream::connect(host)
        .await
        .map_err(ProxyError::IoError)?;

    write_request(&mut BufWriter::new(&mut stream), &proxy_request)
        .await
        .map_err(ProxyError::IoError)?;

    Ok(parse_response(&mut BufReader::new(&mut stream))
        .await
        .map_err(ProxyError::ResponseParseError)?)
}
