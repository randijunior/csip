use std::fmt;
use std::str::{self, Utf8Error};

use thiserror::Error;
use utils::{Position, ScannerError, ScannerErrorKind};

pub type Result<T> = std::result::Result<T, Error>;

impl std::convert::From<Utf8Error> for Error {
    fn from(value: Utf8Error) -> Self {
        Self::Custom(format!("{:#?}", value))
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Parse(#[from] ParseError),

    #[error("Transport: {0}")]
    Transport(String),

    #[error("Transaction Error: {0}")]
    Transaction(#[from] TransactionError),

    #[error("DialogError: {0}")]
    Dialog(String),

    #[error("Missing required '{0}' header")]
    MissingHeader(&'static str),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("Channel closed")]
    ChannelClosed,

    #[error("Unsupported transport")]
    UnsupportedTransport,

    #[error("Invalid Status Code")]
    InvalidStatusCode,

    #[error("Fmt Error: {0}")]
    Fmt(#[from] std::fmt::Error),

    #[error("Resolve Error: {0}")]
    ResolveError(#[from] hickory_resolver::ResolveError),

    #[error("error: {0}")]
    Custom(String),

    #[error("error: {0}")]
    MediaError(#[from] media::error::Error),

    #[error("No Media configured on endpoint!")]
    NoMediaConfigured,
}

impl Error {
    pub fn is_transport_error(&self) -> bool {
        matches!(self, Self::Transport(_))
    }
}

#[derive(Debug, Error)]
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub position: Position,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#?} {:#?}", self.kind, self.position)
    }
}

impl From<ScannerError> for ParseError {
    fn from(value: ScannerError) -> Self {
        Self {
            kind: ParseErrorKind::Scanner(value.kind),
            position: value.position,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum ParseErrorKind {
    #[error("syntax error: {s}")]
    SyntaxError { s: String },
    #[error("invalid port")]
    Port,
    #[error("invalid scheme")]
    Scheme,
    #[error("invalid status code")]
    StatusCode,
    #[error("invalid host")]
    Host,
    #[error("invalid sip method")]
    SipMethod,
    #[error("invalid sip version expected: 'SIP/2.0'")]
    Version,
    #[error("invalid sip uri")]
    Uri,
    #[error("invalid sip parameter")]
    Param,
    #[error("invalid sip transport")]
    Transport,
    #[error("ScannerError: {:#?}", .0)]
    Scanner(ScannerErrorKind),
}

#[derive(Debug, Error, PartialEq)]
pub enum TransactionError {
    #[error(
        "Received invalid 'ACK' method, The ACK request must be passed directly to the transport layer for transmission."
    )]
    AckCannotCreateTransaction,
    #[error("Failed to send request: {0}")]
    FailedToSendMessage(String),
    #[error("Timeout reached after send message")]
    Timeout, //     #[error("The transaction is no longer valid")]
    // Invalid,
    #[error("invalid State")]
    InvalidState,
}
