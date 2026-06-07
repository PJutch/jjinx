use std::{io, net::AddrParseError, num::ParseIntError, string::FromUtf8Error};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Io error: {0}")]
    IoError(io::Error),
    #[error("Utf8 error: {0}")]
    Utf8Error(FromUtf8Error),
    #[error("Parse int error: {0}")]
    ParseIntError(ParseIntError),
    #[error("Address parse error: {0}")]
    AddrParseError(AddrParseError),
    #[error("Unexpected token: {0}, expected {1}")]
    UnexpectedToken(String, String),
    #[error("Unexpected eof")]
    UnexpectedEof,
    #[error("Unknown field: {0}")]
    UnknownField(String),
    #[error("Duplicate field: {0}")]
    DuplicateField(String),
    #[error("Route {0} already has content")]
    DuplicateContent(String),
    #[error("Upstream {0} already has balancing mode")]
    DuplicateBalancing(String),
    #[error("Unknown matcher: {0}")]
    UnknownMatcher(String),
    #[error("Unknow escape sequence: \\{0}")]
    UnknownEscape(String),
}
