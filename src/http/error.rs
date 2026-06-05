use thiserror::Error;

use std::io;
use std::num::ParseIntError;
use std::string::FromUtf8Error;

use super::HttpVersion;

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Unexpected eof")]
    UnexpectedEof,
    #[error("Io error: {0}")]
    IoError(io::Error),
    #[error("Utf8 error: {0}")]
    Utf8Error(FromUtf8Error),
    #[error("Expected a digit")]
    ExpectedDigit,
    #[error("Parse int error: {0}")]
    ParseIntError(ParseIntError),
    #[error("Error parsing HTTP version")]
    ParseVersionError,
    #[error("Invalid HTTP version")]
    InvalidHttpVersion(HttpVersion),
    #[error("Expected space")]
    ExpectedSpace,
    #[error("Expected newline")]
    ExpectedNewline,
    #[error("Expected colon after field name")]
    ExpectedColon,
    #[error("Expected line feed after carraige return")]
    ExpectedLineFeed,
    #[error("Cut percent encoding")]
    CutPersonEncoding,
    #[error("Percent encoding should have hex digits, not byte value {0}")]
    InvalidParcentEncodingDigit(u8),
}
