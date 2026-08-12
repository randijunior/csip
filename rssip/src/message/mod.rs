//! SIP Message types

pub mod auth;
pub mod headers;
pub mod media_type;
pub mod method;
pub mod params;
pub mod status_code;
pub mod uri;

use std::{borrow, fmt, ops};

use crate::message::headers::Headers;
use crate::message::method::SipMethod;
use crate::message::status_code::StatusCode;
use crate::message::uri::Uri;

/// An SIP message, either `Request` or `Response`.
pub enum SipMessage {
    /// An SIP Request.
    Request(Request),
    /// An SIP Response.
    Response(Response),
}

/// A SIP Request.
#[derive(Clone)]
pub struct Request {
    pub req_line: RequestLine,
    pub headers: Headers,
    pub body: Option<SipBody>,
}

/// A SIP `Request-Line`.
#[derive(Clone)]
pub struct RequestLine {
    pub method: SipMethod,
    pub uri: Uri,
}

/// A SIP Response.
#[derive(Clone)]
pub struct Response {
    pub status_line: StatusLine,
    pub headers: Headers,
    pub body: Option<SipBody>,
}

/// A SIP Status-Line.
#[derive(Clone)]
pub struct StatusLine {
    pub code: StatusCode,
    pub reason: ReasonPhrase,
}

/// A SIP reason-phrase.
#[derive(Clone)]
pub struct ReasonPhrase(borrow::Cow<'static, str>);

/// This type represents a body in a SIP message.
#[derive(Clone, Default, Debug)]
pub struct SipBody {
    data: bytes::Bytes,
}

impl SipMessage {
    /// Returns a reference to the contained [`Request`] value if is
    /// an request variant.
    pub fn request(&self) -> Option<&Request> {
        if let SipMessage::Request(request) = self {
            Some(request)
        } else {
            None
        }
    }

    /// Returns a reference to the contained [`Response`] value if is
    /// an response variant.
    pub fn response(&self) -> Option<&Response> {
        if let SipMessage::Response(response) = self {
            Some(response)
        } else {
            None
        }
    }

    /// Returns a reference to the headers of the message.
    pub fn headers(&self) -> &Headers {
        match self {
            SipMessage::Request(request) => &request.headers,
            SipMessage::Response(response) => &response.headers,
        }
    }

    /// Returns a mutable reference to the of the message.
    pub fn headers_mut(&mut self) -> &mut Headers {
        match self {
            SipMessage::Request(req) => &mut req.headers,
            SipMessage::Response(res) => &mut res.headers,
        }
    }

    /// Returns a reference to the message body.
    pub fn body(&self) -> Option<&SipBody> {
        match self {
            SipMessage::Request(request) => request.body.as_ref(),
            SipMessage::Response(response) => response.body.as_ref(),
        }
    }

    /// Returns a mutable reference to the message body.
    pub fn body_mut(&mut self) -> &mut Option<SipBody> {
        match self {
            SipMessage::Request(request) => &mut request.body,
            SipMessage::Response(response) => &mut response.body,
        }
    }
}

impl From<Request> for SipMessage {
    fn from(request: Request) -> Self {
        SipMessage::Request(request)
    }
}

impl From<Response> for SipMessage {
    fn from(response: Response) -> Self {
        SipMessage::Response(response)
    }
}

impl Request {
    /// Creates a new SIP `Request`.
    pub fn new(method: SipMethod, uri: Uri) -> Self {
        Self {
            req_line: RequestLine { method, uri },
            headers: Headers::new(),
            body: None,
        }
    }

    /// Create a new request with custom headers.
    pub fn with_headers(method: SipMethod, uri: Uri, headers: Headers) -> Self {
        Self {
            req_line: RequestLine { method, uri },
            headers,
            body: None,
        }
    }
}

impl fmt::Display for RequestLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} SIP/2.0\r\n", self.method, self.uri)
    }
}

impl Response {
    /// Creates a new SIP `Response`.
    pub fn new(code: StatusCode, reason: ReasonPhrase) -> Self {
        Self {
            status_line: StatusLine { code, reason },
            headers: Headers::new(),
            body: None,
        }
    }

    /// Create a new response with custom headers.
    pub const fn with_headers(status_line: StatusLine, headers: Headers) -> Self {
        Self {
            status_line,
            headers,
            body: None,
        }
    }
}

impl fmt::Display for StatusLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SIP/2.0 {} {}\r\n", self.code as u32, self.reason.0)
    }
}

impl ReasonPhrase {
    /// Creates a new `ReasonPhrase`.
    #[inline]
    pub const fn new(reason: borrow::Cow<'static, str>) -> Self {
        Self(reason)
    }

    /// Returns the inner phrase as str.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub const fn from_static(s: &'static str) -> Self {
        Self(borrow::Cow::Borrowed(s))
    }
}

impl<S> From<S> for ReasonPhrase
where
    S: Into<borrow::Cow<'static, str>>,
{
    fn from(value: S) -> Self {
        Self::new(value.into())
    }
}

impl SipBody {
    /// Creates a new `SipBody` whith the given `data`.
    #[inline]
    pub fn new(data: bytes::Bytes) -> Self {
        Self { data }
    }
}

impl From<&str> for SipBody {
    fn from(value: &str) -> Self {
        value.as_bytes().into()
    }
}

impl From<&[u8]> for SipBody {
    fn from(data: &[u8]) -> Self {
        Self::new(bytes::Bytes::copy_from_slice(data))
    }
}

impl From<bytes::Bytes> for SipBody {
    fn from(data: bytes::Bytes) -> Self {
        Self::new(data)
    }
}

impl ops::Deref for SipBody {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}
