use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use std::u32;

use rustls::pki_types::{DnsName, InvalidDnsNameError, ServerName};
use scopeguard::defer;
use std::io;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, BufReader, BufWriter};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use twox_hash::XxHash64;

use crate::config;
use crate::http::{self, HttpVersion, Request, Response, SendError, parse_response, write_request};
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
    #[error("Timeout")]
    Timeout,
    #[error("Invalid domain name")]
    DomainNameError(InvalidDnsNameError),
}

enum Balancer {
    RoundRobin(AtomicUsize),
    LeastConnected(Vec<AtomicU32>),
    IpHash,
}

pub struct Upstream {
    balancer: Balancer,
    servers: Vec<String>,

    failures: Vec<Mutex<VecDeque<Instant>>>,
    max_fails: usize,

    connect_timeout: Duration,
    fail_timeout: Duration,
    header_timeout: Duration,
    body_timeout: Duration,
    send_timeout: Duration,
}

impl Upstream {
    fn add_failure(&self, server: usize) {
        match self.failures[server].lock() {
            Ok(mut failures) => {
                failures.push_back(Instant::now());
            }
            Err(err) => {
                err.into_inner().clear();
            }
        }
    }

    fn update_failures(&self, server: usize) -> bool {
        match self.failures[server].lock() {
            Ok(mut failures) => {
                while failures
                    .front()
                    .is_some_and(|instant| instant.elapsed() > self.fail_timeout)
                {
                    failures.pop_front();
                }

                failures.len() < self.max_fails
            }
            Err(err) => {
                err.into_inner().clear();
                true
            }
        }
    }

    fn get_server(&self, ip: IpAddr) -> usize {
        match &self.balancer {
            Balancer::RoundRobin(next_server) => {
                let mut current_server = next_server.load(Ordering::Relaxed);
                for _ in 0..self.servers.len() + 1 {
                    loop {
                        match next_server.compare_exchange_weak(
                            current_server,
                            (current_server + 1) % self.servers.len(),
                            Ordering::Relaxed,
                            Ordering::Relaxed,
                        ) {
                            Ok(_) => break,
                            Err(new_current_server) => current_server = new_current_server,
                        }
                    }

                    if self.update_failures(current_server) {
                        return current_server;
                    }
                }
                current_server
            }
            Balancer::LeastConnected(connection_counts) => {
                let mut best_server = 0;
                let mut best_count = u32::MAX;

                let mut best_failed = 0;
                let mut best_failed_count = u32::MAX;

                for (i, count) in connection_counts.iter().enumerate() {
                    if self.update_failures(i) {
                        let count = count.load(Ordering::Relaxed);
                        if count < best_count {
                            best_server = i;
                            best_count = count;
                        }
                    } else {
                        let count = count.load(Ordering::Relaxed);
                        if count < best_failed_count {
                            best_failed = i;
                            best_failed_count = count;
                        }
                    }
                }

                if best_count == u32::MAX {
                    best_server = best_failed;
                }

                connection_counts[best_server].fetch_add(1, Ordering::Relaxed);

                best_server
            }
            Balancer::IpHash => {
                let mut hasher = XxHash64::default();
                ip.hash(&mut hasher);
                let server = hasher.finish() as usize % self.servers.len();

                self.update_failures(server);

                server
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
                    failures: vec![VecDeque::new(); config.servers.len()]
                        .into_iter()
                        .map(Mutex::new)
                        .collect(),
                    max_fails: config.max_fails.unwrap_or(1),
                    fail_timeout: config.fail_timeout.unwrap_or(Duration::from_secs(10)),
                    connect_timeout: config.connect_timeout.unwrap_or(Duration::from_secs(60)),
                    header_timeout: config.header_timeout.unwrap_or(Duration::from_secs(60)),
                    body_timeout: config.body_timeout.unwrap_or(Duration::from_secs(60)),
                    send_timeout: config.send_timeout.unwrap_or(Duration::from_secs(60)),
                    servers: config.servers,
                },
            )
        })
        .collect()
}

async fn exchange_data<Stream: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut Stream,
    request: &Request,
    send_timeout: Duration,
    header_timeout: Duration,
    body_timeout: Duration,
) -> Result<Response, ProxyError> {
    write_request(&mut BufWriter::new(&mut *stream), &request, send_timeout)
        .await
        .map_err(|err| match err {
            SendError::IoError(err) => ProxyError::IoError(err),
            SendError::Timeout => ProxyError::Timeout,
        })?;

    Ok(
        parse_response(&mut BufReader::new(stream), header_timeout, body_timeout)
            .await
            .map_err(ProxyError::ResponseParseError)?,
    )
}

async fn proxy_pass_to(
    uri: &Uri,
    host: &str,
    request: &Request,
    matching_prefix_len: usize,
    new_headers: HashMap<String, String>,
    connect_timeout: Duration,
    header_timeout: Duration,
    body_timeout: Duration,
    send_timeout: Duration,
    connector: Option<&TlsConnector>,
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

    let mut socket = timeout(connect_timeout, TcpStream::connect(host))
        .await
        .map_err(|_| ProxyError::Timeout)?
        .map_err(ProxyError::IoError)?;

    let domain = &host[..host.find(':').unwrap_or(host.len())];
    let domain = if let Ok(ip) = domain.parse::<IpAddr>() {
        ServerName::IpAddress(ip.into())
    } else {
        ServerName::DnsName(
            DnsName::try_from_str(domain)
                .map_err(ProxyError::DomainNameError)?
                .to_owned(),
        )
    };

    if let Some(connector) = connector {
        let mut stream = connector
            .connect(domain, socket)
            .await
            .map_err(ProxyError::IoError)?;

        exchange_data(
            &mut BufWriter::new(&mut stream),
            &proxy_request,
            send_timeout,
            header_timeout,
            body_timeout,
        )
        .await
    } else {
        exchange_data(
            &mut BufWriter::new(&mut socket),
            &proxy_request,
            send_timeout,
            header_timeout,
            body_timeout,
        )
        .await
    }
}

pub async fn proxy_pass(
    uri: &Uri,
    request: &Request,
    matching_prefix_len: usize,
    new_headers: HashMap<String, String>,
    upstreams: Arc<HashMap<String, Upstream>>,
    connector: TlsConnector,
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

    match proxy_pass_to(
        &uri,
        host,
        request,
        matching_prefix_len,
        new_headers,
        upstream
            .map(|pair| pair.0.connect_timeout)
            .unwrap_or(Duration::from_secs(60)),
        upstream
            .map(|pair| pair.0.header_timeout)
            .unwrap_or(Duration::from_secs(60)),
        upstream
            .map(|pair| pair.0.body_timeout)
            .unwrap_or(Duration::from_secs(60)),
        upstream
            .map(|pair| pair.0.send_timeout)
            .unwrap_or(Duration::from_secs(60)),
        if uri.scheme() == "https" {
            Some(&connector)
        } else {
            None
        },
    )
    .await
    {
        Ok(response) => Ok(response),
        Err(err) => {
            if let Some((upstream, server)) = upstream {
                upstream.add_failure(server);
            }
            Err(err)
        }
    }
}
