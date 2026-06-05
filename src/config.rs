use std::{collections::HashMap, net::Ipv4Addr};

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
}

#[derive(Debug, Default, PartialEq)]
pub struct Server {
    pub port: Option<u16>,
    pub ip: Option<Ipv4Addr>,
    pub domain_names: Vec<String>,
    pub is_default: bool,
    pub routes: Vec<Route>,
}
