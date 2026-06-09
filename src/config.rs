use std::{collections::HashMap, net::IpAddr, path::PathBuf, time::Duration};

mod error;
pub use error::ParseError;

mod tokenizer;

mod parse;
pub use parse::parse_config;

#[derive(Debug, Default, PartialEq)]
pub enum Content {
    #[default]
    NoContent,
    RawData(String),
    FileAny(Vec<String>),
    Redirect(String),
    Proxy {
        uri: String,
        headers: HashMap<String, String>,
    },
}

#[derive(Debug, Default, PartialEq)]
pub enum UriMatcher {
    #[default]
    Prefix,
    Regex,
    PrefixPrioritiesed,
    Exact,
}

#[derive(Debug, Default, PartialEq)]
pub struct Route {
    pub uri: String,
    pub matcher: UriMatcher,
    pub content: Option<Content>,
    pub status: Option<i16>,
    pub headers: HashMap<String, String>,
    pub default_content_type: Option<String>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Server {
    pub port: Option<u16>,
    pub ip: Option<IpAddr>,
    pub domain_names: Vec<String>,
    pub is_default: bool,

    pub routes: Vec<Route>,

    pub header_timeout: Option<Duration>,
    pub body_timeout: Option<Duration>,
    pub send_timeout: Option<Duration>,

    pub cert_path: Option<PathBuf>,
    pub keys_path: Option<PathBuf>,

    pub error_pages: HashMap<i16, String>,
}

#[derive(Debug, PartialEq)]
pub struct ServerGroup {
    pub servers: Vec<Server>,
    pub default: usize,
}

#[derive(Debug, PartialEq)]
pub enum Balancing {
    RoundRobin,
    LeastConnected,
    IpHash,
}

#[derive(Debug, Default, PartialEq)]
pub struct Upstream {
    pub servers: Vec<String>,
    pub balancing: Option<Balancing>,

    pub max_fails: Option<usize>,
    pub fail_timeout: Option<Duration>,

    pub connect_timeout: Option<Duration>,
    pub header_timeout: Option<Duration>,
    pub body_timeout: Option<Duration>,
    pub send_timeout: Option<Duration>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Config {
    pub servers: HashMap<(IpAddr, u16), ServerGroup>,
    pub upstreams: HashMap<String, Upstream>,
}
