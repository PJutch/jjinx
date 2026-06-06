use crate::uri::Uri;
use std::collections::HashMap;

mod error;
pub use error::ParseError;

mod io;

mod parse;

mod request_parse;
pub use request_parse::parse_request;

mod request_write;
pub use request_write::write_request;

mod response_parse;
pub use response_parse::parse_response;

mod response_write;
pub use response_write::write_response;

#[derive(PartialEq, Debug, Clone, Copy)]
pub struct HttpVersion(pub u8, pub u8);

#[derive(PartialEq, Debug, Clone)]
pub struct StartLine {
    pub method: String,
    pub uri: Uri,
    pub version: HttpVersion,
}

#[derive(PartialEq, Debug, Clone)]
pub struct Request {
    pub start_line: StartLine,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

pub struct Response {
    pub status: i16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}
