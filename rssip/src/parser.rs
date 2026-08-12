//! SIP Parser

use std::str::{self, FromStr};

use utils::lookup_table::LookupTable;
use utils::scanner::Scanner;
use utils::{lookup_table, scanner};

use crate::error::{Error, ParseError, ParseErrorKind as Kind, Result};
use crate::message::headers::{self as header, Header};
use crate::message::{self, auth, method, params, status_code, uri};
use crate::{macros, transport};

#[must_use]
pub fn is_via_param(b: u8) -> bool {
    VIA_PARAM_TAB[b as usize]
}

#[must_use]
pub fn is_host(b: u8) -> bool {
    HOST_TAB[b as usize]
}

#[must_use]
pub fn is_token(b: u8) -> bool {
    TOKEN_TAB[b as usize]
}

pub const SIPV2: &str = "SIP/2.0";
const ALPHANUMERIC: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const UNRESERVED: &str = "-_.!~*'()%";
const ESCAPED: &str = "%";
const USER_UNRESERVED: &str = "&=+$,;?/";
const TOKEN: &str = "-.!%*_`'~+";
const PASS: &str = "&=+$,";
const HOST: &str = "_-.";
const SIP: &[u8] = b"sip";
const SIPS: &[u8] = b"sips";
const B_SIPV2: &[u8] = SIPV2.as_bytes();

const TOKEN_TAB: LookupTable = lookup_table!(ALPHANUMERIC, TOKEN);
const USER_TAB: LookupTable = lookup_table!(ALPHANUMERIC, UNRESERVED, USER_UNRESERVED, ESCAPED);
const PASS_TAB: LookupTable = lookup_table!(ALPHANUMERIC, UNRESERVED, ESCAPED, PASS);
const HOST_TAB: LookupTable = lookup_table!(ALPHANUMERIC, HOST);
const PARAM_TAB: LookupTable = lookup_table!("[]/:&+$", ALPHANUMERIC, UNRESERVED, ESCAPED);
const HDR_TAB: LookupTable = lookup_table!("[]/?:+$", ALPHANUMERIC, UNRESERVED, ESCAPED);
const VIA_PARAM_TAB: LookupTable = lookup_table!("[:]", ALPHANUMERIC, TOKEN);

/// A SIP message parser.
pub struct SipParser<'buf> {
    scanner: Scanner<'buf>,
}

/// Trait to parse SIP headers.
pub trait HeaderParse {
    /// The full name of the SIP header.
    const NAME: &'static str;
    /// The abbreviated name of the SIP header, if any.
    const SHORT_NAME: &'static str = panic!("This header not have a short name!");

    /// Parse the SIP header
    fn parse(parser: &mut SipParser) -> Result<Self>
    where
        Self: Sized;
}

impl<'buf> SipParser<'buf> {
    /// Creates a new `SipParser` from the given scanner slice.
    #[inline]
    pub fn new<B>(buf: &'buf B) -> Self
    where
        B: AsRef<[u8]> + ?Sized,
    {
        Self {
            scanner: Scanner::new(buf.as_ref()),
        }
    }

    /// Parses the `buf` into a [`SipMessage`].
    #[inline]
    pub fn parse<B>(buf: &'buf B) -> Result<message::SipMessage>
    where
        B: AsRef<[u8]> + ?Sized,
    {
        Self::new(buf.as_ref()).parse_sip_msg()
    }

    /// Parses the internal buffer into a [`SipMessage`].
    pub fn parse_sip_msg(&mut self) -> Result<message::SipMessage> {
        // Might be enough for most messages.
        let minimal_header_size = 7;

        let is_sip_v2 = matches!(self.scanner.peek_n(B_SIPV2.len()), Some(B_SIPV2));

        let mut sip_message = if is_sip_v2 {
            // Is an status line, e.g, "SIP/2.0 200 OK".
            let status_line = self.parse_status_line()?;
            let headers = header::Headers::with_capacity(minimal_header_size);

            message::SipMessage::Response(message::Response {
                status_line,
                headers,
                body: None,
            })
        } else {
            // Is an request line, e.g, "OPTIONS sip:localhost SIP/2.0".
            let req_line = self.parse_request_line()?;
            let headers = header::Headers::with_capacity(minimal_header_size);

            message::SipMessage::Request(message::Request {
                req_line,
                headers,
                body: None,
            })
        };

        let mut found_content_type_hdr = false;
        let headers = sip_message.headers_mut();

        'headers: loop {
            // Get name.
            let header_name = self.header_name()?;

            match header_name {
                header::ErrorInfo::NAME => {
                    let header = self.parse_header::<header::ErrorInfo>()?;
                    headers.push(Header::ErrorInfo(header));
                }
                header::Route::NAME => loop {
                    let header = self.parse_header::<header::Route>()?;
                    headers.push(Header::Route(header));

                    if self.take_if_eq(b',').is_none() {
                        break;
                    }
                },
                header::Via::NAME | header::Via::SHORT_NAME => loop {
                    let header = self.parse_header::<header::Via>()?;
                    headers.push(Header::Via(header));

                    if self.take_if_eq(b',').is_none() {
                        break;
                    }
                },
                header::MaxForwards::NAME => {
                    let header = self.parse_header::<header::MaxForwards>()?;
                    headers.push(Header::MaxForwards(header));
                }
                header::From::NAME | header::From::SHORT_NAME => {
                    let header = self.parse_header::<header::From>()?;
                    headers.push(Header::From(header));
                }
                header::To::NAME | header::To::SHORT_NAME => {
                    let header = self.parse_header::<header::To>()?;
                    headers.push(Header::To(header));
                }
                header::CallId::NAME | header::CallId::SHORT_NAME => {
                    let header = self.parse_header::<header::CallId>()?;
                    headers.push(Header::CallId(header));
                }
                header::CSeq::NAME => {
                    let header = self.parse_header::<header::CSeq>()?;
                    headers.push(Header::CSeq(header));
                }
                header::Authorization::NAME => {
                    let header = self.parse_header::<header::Authorization>()?;
                    headers.push(Header::Authorization(header));
                }
                header::Contact::NAME | header::Contact::SHORT_NAME => loop {
                    let header = self.parse_header::<header::Contact>()?;
                    headers.push(Header::Contact(header));

                    if self.take_if_eq(b',').is_none() {
                        break;
                    }
                },
                header::Expires::NAME => {
                    let header = self.parse_header::<header::Expires>()?;
                    headers.push(Header::Expires(header));
                }
                header::InReplyTo::NAME => {
                    let header = self.parse_header::<header::InReplyTo>()?;
                    headers.push(Header::InReplyTo(header));
                }
                header::MimeVersion::NAME => {
                    let header = self.parse_header::<header::MimeVersion>()?;
                    headers.push(Header::MimeVersion(header));
                }
                header::MinExpires::NAME => {
                    let header = self.parse_header::<header::MinExpires>()?;
                    headers.push(Header::MinExpires(header));
                }
                header::UserAgent::NAME => {
                    let header = self.parse_header::<header::UserAgent>()?;
                    headers.push(Header::UserAgent(header));
                }
                header::Date::NAME => {
                    let header = self.parse_header::<header::Date>()?;
                    headers.push(Header::Date(header));
                }
                header::Server::NAME => {
                    let header = self.parse_header::<header::Server>()?;
                    headers.push(Header::Server(header));
                }
                header::Subject::NAME | header::Subject::SHORT_NAME => {
                    let header = self.parse_header::<header::Subject>()?;
                    headers.push(Header::Subject(header));
                }
                header::Priority::NAME => {
                    let header = self.parse_header::<header::Priority>()?;
                    headers.push(Header::Priority(header));
                }
                header::ProxyAuthenticate::NAME => {
                    let header = self.parse_header::<header::ProxyAuthenticate>()?;
                    headers.push(Header::ProxyAuthenticate(header));
                }
                header::ProxyAuthorization::NAME => {
                    let header = self.parse_header::<header::ProxyAuthorization>()?;
                    headers.push(Header::ProxyAuthorization(header));
                }
                header::ProxyRequire::NAME => {
                    let header = self.parse_header::<header::ProxyRequire>()?;
                    headers.push(Header::ProxyRequire(header));
                }
                header::ReplyTo::NAME => {
                    let header = self.parse_header::<header::ReplyTo>()?;
                    headers.push(Header::ReplyTo(header));
                }
                header::ContentLength::NAME | header::ContentLength::SHORT_NAME => {
                    let header = self.parse_header::<header::ContentLength>()?;
                    headers.push(Header::ContentLength(header));
                }
                header::ContentEncoding::NAME | header::ContentEncoding::SHORT_NAME => {
                    let header = self.parse_header::<header::ContentEncoding>()?;
                    headers.push(Header::ContentEncoding(header));
                }
                header::ContentType::NAME | header::ContentType::SHORT_NAME => {
                    let header = self.parse_header::<header::ContentType>()?;
                    headers.push(Header::ContentType(header));
                    found_content_type_hdr = true;
                }
                header::ContentDisposition::NAME => {
                    let header = self.parse_header::<header::ContentDisposition>()?;
                    headers.push(Header::ContentDisposition(header));
                }
                header::RecordRoute::NAME => loop {
                    let header = self.parse_header::<header::RecordRoute>()?;
                    headers.push(Header::RecordRoute(header));

                    if self.take_if_eq(b',').is_none() {
                        break;
                    }
                },
                header::Require::NAME => {
                    let header = self.parse_header::<header::Require>()?;
                    headers.push(Header::Require(header));
                }
                header::RetryAfter::NAME => {
                    let header = self.parse_header::<header::RetryAfter>()?;
                    headers.push(Header::RetryAfter(header));
                }
                header::Organization::NAME => {
                    let header = self.parse_header::<header::Organization>()?;
                    headers.push(Header::Organization(header));
                }
                header::AcceptEncoding::NAME => {
                    let header = self.parse_header::<header::AcceptEncoding>()?;
                    headers.push(Header::AcceptEncoding(header));
                }
                header::Accept::NAME => {
                    let header = self.parse_header::<header::Accept>()?;
                    headers.push(Header::Accept(header));
                }
                header::AcceptLanguage::NAME => {
                    let header = self.parse_header::<header::AcceptLanguage>()?;
                    headers.push(Header::AcceptLanguage(header));
                }
                header::AlertInfo::NAME => {
                    let header = self.parse_header::<header::AlertInfo>()?;
                    headers.push(Header::AlertInfo(header));
                }
                header::Allow::NAME => {
                    let header = self.parse_header::<header::Allow>()?;
                    headers.push(Header::Allow(header));
                }
                header::AuthenticationInfo::NAME => {
                    let header = self.parse_header::<header::AuthenticationInfo>()?;
                    headers.push(Header::AuthenticationInfo(header));
                }
                header::Supported::NAME | header::Supported::SHORT_NAME => {
                    let header = self.parse_header::<header::Supported>()?;
                    headers.push(Header::Supported(header));
                }
                header::Timestamp::NAME => {
                    let header = self.parse_header::<header::Timestamp>()?;
                    headers.push(Header::Timestamp(header));
                }
                header::Unsupported::NAME => {
                    let header = self.parse_header::<header::Unsupported>()?;
                    headers.push(Header::Unsupported(header));
                }
                header::WWWAuthenticate::NAME => {
                    let header = self.parse_header::<header::WWWAuthenticate>()?;
                    headers.push(Header::WWWAuthenticate(header));
                }
                header::Warning::NAME => {
                    let header = self.parse_header::<header::Warning>()?;
                    headers.push(Header::Warning(header));
                }
                name => {
                    // Found a header that is not defined in RFC 3261.
                    let data = self.read_line()?;
                    let header = header::RawHeader::new(name, data);
                    headers.push(Header::RawHeader(header));
                }
            };

            self.scanner.scan_newline().map_err(ParseError::from)?;

            if matches!(self.scanner.peek(), Some(b'\r') | Some(b'\n') | None) {
                break 'headers;
            }
        }

        if found_content_type_hdr {
            self.skip_newline();
            let body = self.scanner.remaining();
            *sip_message.body_mut() = Some(body.into());
        }

        Ok(sip_message)
    }

    pub fn parse_status_line(&mut self) -> Result<message::StatusLine> {
        self.parse_sip_version()?;

        let code = self.parse_status_code()?;
        let reason = self.parse_reason_phrase()?;

        self.skip_newline();

        Ok(message::StatusLine { code, reason })
    }

    pub fn parse_request_line(&mut self) -> Result<message::RequestLine> {
        let token = self.scanner.scan_while(is_token);

        let method = token.into();
        self.skip_ws();
        let uri = self.parse_uri(true)?;
        self.skip_ws();
        self.parse_sip_version()?;

        self.skip_newline();

        Ok(message::RequestLine { method, uri })
    }

    #[inline]
    pub fn parse_sip_version(&mut self) -> Result<()> {
        self.scanner
            .must_scan_slice(B_SIPV2)
            .or_else(|_| self.error(Kind::Version))
    }

    #[must_use]
    fn parse_header<H: HeaderParse>(&mut self) -> Result<H> {
        <H as HeaderParse>::parse(self)
    }

    fn header_name(&mut self) -> Result<&'buf str> {
        let header_name = self.read_token();

        self.skip_ws();
        self.must_read(b':')?;
        self.skip_ws();

        Ok(header_name)
    }

    pub fn parse_sip_uri(&mut self, params: bool) -> Result<uri::SipUri> {
        self.skip_ws();

        if matches!(self.scanner.peek(), Some(b'"') | Some(b'<')) {
            let name_addr = self.parse_name_addr()?;
            Ok(uri::SipUri::NameAddr(name_addr))
        } else {
            let scheme_or_display = self
                .scanner
                .peek_while(|b| !matches!(b, b':' | b'\r' | b'\n' | b'<'));
            if matches!(scheme_or_display, SIP | SIPS) {
                let uri = self.parse_uri(params)?;
                Ok(uri::SipUri::Uri(uri))
            } else {
                let name_addr = self.parse_name_addr()?;
                Ok(uri::SipUri::NameAddr(name_addr))
            }
        }
    }

    pub fn parse_uri(&mut self, parse_params: bool) -> Result<uri::Uri> {
        let mut uri = message::uri::Uri {
            scheme: self.parse_scheme()?,
            user: self.parse_user_info()?,
            host_port: self.parse_host_port()?,
            ..Default::default()
        };

        if !parse_params {
            return Ok(uri);
        }

        // Parse SIP uri parameters.
        uri.params = macros::parse_params!(self, {
            let (pname, pvalue) = self.parse_uri_param()?;

            match pname {
                params::USER_PARAM => {
                    uri.user_param = pvalue.map(ToOwned::to_owned);
                    None
                }
                params::METHOD_PARAM => {
                    uri.method_param = pvalue.map(method::SipMethod::from);
                    None
                }
                params::TRANSPORT_PARAM => {
                    uri.transport_param = pvalue
                        .map(transport::TransportProtocol::from_str)
                        .transpose()
                        .or_else(|_| self.error(Kind::Transport))?;
                    None
                }
                params::TTL_PARAM => {
                    uri.ttl_param = pvalue
                        .map(|ttl| ttl.parse())
                        .transpose()
                        .or_else(|_| self.error(Kind::Param))?;
                    None
                }
                params::LR_PARAM => {
                    uri.lr_param = true;
                    None
                }
                params::MADDR_PARAM => {
                    uri.maddr_param = pvalue
                        .map(|maddr| maddr.parse::<uri::Host>())
                        .transpose()
                        .or_else(|_| self.error(Kind::Host))?;
                    None
                }
                _ => Some((pname, pvalue).into()),
            }
        });

        uri.headers = if self.take_if_eq(b'?').is_some() {
            // The uri has header parameters.
            Some(self.parse_uri_headers()?)
        } else {
            None
        };

        Ok(uri)
    }

    pub fn parse_name_addr(&mut self) -> Result<uri::NameAddr> {
        self.skip_ws();
        let display = self.parse_display_name()?;
        self.skip_ws();

        self.must_read(b'<')?;
        let uri = self.parse_uri(true)?;
        self.must_read(b'>')?;

        Ok(uri::NameAddr { display, uri })
    }

    pub fn parse_host_port(&mut self) -> Result<uri::HostPort> {
        let host = match self.peek() {
            Some(b'[') => {
                // Is a Ipv6 host
                self.advance()?;
                // the '[' and ']' characters are removed from the host
                let host = self
                    .scanner
                    .scan_while_as_str(|b| b != b']')
                    .or_else(|_| self.error(Kind::Host))?;
                self.advance()?;

                if let Ok(ipv6_addr) = host.parse() {
                    uri::Host::IpAddr(ipv6_addr)
                } else {
                    return self.error(Kind::Host);
                }
            }
            _ => {
                // Is a domain name or Ipv4 host.
                let host = self.read_host();
                if host.is_empty() {
                    return self.error(Kind::Host);
                }
                if let Ok(ip_addr) = host.parse() {
                    uri::Host::IpAddr(ip_addr)
                } else {
                    uri::Host::HostName(uri::HostName::from(host))
                }
            }
        };

        let port = self.parse_port()?;

        Ok(uri::HostPort { host, port })
    }

    fn parse_status_code(&mut self) -> Result<status_code::StatusCode> {
        self.skip_ws();
        let digits = self.scanner.scan_while(|b| b.is_ascii_digit());
        self.skip_ws();

        let code = digits
            .try_into()
            .or_else(|_| self.error(Kind::StatusCode))?;

        Ok(code)
    }

    fn parse_reason_phrase(&mut self) -> Result<message::ReasonPhrase> {
        let reason = self.read_line()?.to_owned();

        Ok(message::ReasonPhrase::from(reason))
    }

    fn parse_scheme(&mut self) -> Result<uri::Scheme> {
        let token = self.scanner.peek_while(is_token);

        let scheme = match token {
            SIP => uri::Scheme::Sip,
            SIPS => uri::Scheme::Sips,
            _ => return self.error(Kind::Scheme),
        };

        // Eat the scheme.
        self.scanner.advance_n(token.len());

        // Eat the ":" character.
        self.must_read(b':')?;

        Ok(scheme)
    }

    fn parse_user_info(&mut self) -> Result<Option<uri::UserInfo>> {
        if self.exists_user_part_in_uri() {
            // We have user part in uri.
            let user = self.read_user().to_owned();
            let pass = if self.take_if_eq(b':').is_some() {
                Some(self.read_pass().to_owned())
            } else {
                None
            };
            // Take '@'.
            self.scanner
                .must_read(b'@')
                .or_else(|_| self.error(Kind::Uri))?;

            Ok(Some(uri::UserInfo { user, pass }))
        } else {
            Ok(None)
        }
    }

    fn parse_port(&mut self) -> Result<Option<u16>> {
        if self.take_if_eq(b':').is_some() {
            self.scanner
                .scan_u16()
                .or_else(|_| self.error(Kind::Host))
                .and_then(|port| {
                    if crate::is_valid_port(port) {
                        Ok(Some(port))
                    } else {
                        self.error(Kind::Host)
                    }
                })
        } else {
            Ok(None)
        }
    }

    fn parse_uri_headers(&mut self) -> Result<uri::UriHeaders> {
        let mut uri_headers = uri::UriHeaders::default();
        loop {
            let param = self.parse_uri_header()?;
            uri_headers.push(param);

            if self.take_if_eq(b'&').is_none() {
                break;
            }
        }
        Ok(uri_headers)
    }

    fn parse_display_name(&mut self) -> Result<Option<uri::DisplayName>> {
        match self.peek() {
            Some(b'"') => {
                self.advance()?; // consume '"'
                let name = self.scanner.scan_while(|b| b != b'"');
                self.advance()?; // consume closing '"'
                let name = str::from_utf8(name)?.to_owned();

                Ok(Some(uri::DisplayName::new(name)))
            }
            Some(b'<') => Ok(None), // no display name
            _ => {
                let name = self.read_token();
                self.skip_ws();
                Ok(Some(uri::DisplayName::new(name.to_owned())))
            }
        }
    }

    fn exists_user_part_in_uri(&self) -> bool {
        self.scanner
            .remaining()
            .iter()
            .take_while(|&&b| !scanner::is_space(b) && !scanner::is_newline(b) && b != b'>')
            .any(|&b| b == b'@')
    }

    #[inline]
    pub fn token(&mut self) -> Result<&'buf str> {
        if let Some(b'"') = self.take_if_eq(b'"') {
            let value = self.scanner.scan_while(|b| b != b'"');
            self.advance()?;

            Ok(str::from_utf8(value)?)
        } else {
            Ok(self.read_token())
        }
    }

    /// Shortcut for yielding a parse error wrapped in a result type.
    pub fn error<T>(&self, kind: Kind) -> Result<T> {
        Err(Error::Parse(ParseError {
            kind,
            position: self.scanner.position(),
        }))
    }

    pub fn parse_auth_challenge(&mut self) -> Result<auth::Challenge> {
        let scheme = self.token()?;

        if scheme == auth::DIGEST {
            return self.parse_digest_challenge();
        }

        let params = macros::parse_params!(self);

        Ok(auth::Challenge::Other {
            scheme: scheme.to_owned(),
            params,
        })
    }

    fn parse_digest_challenge(&mut self) -> Result<auth::Challenge> {
        let mut digest = auth::DigestChallenge::default();

        loop {
            self.skip_ws();
            let (name, value) = self.param_ref()?;

            match name {
                auth::REALM => digest.realm = value.map(String::from),
                auth::NONCE => digest.nonce = value.map(String::from),
                auth::DOMAIN => digest.domain = value.map(String::from),
                auth::ALGORITHM => digest.algorithm = value.map(String::from),
                auth::OPAQUE => digest.opaque = value.map(String::from),
                auth::QOP => digest.qop = value.map(String::from),
                auth::STALE => digest.stale = value.map(String::from),
                _other => {
                    // return err?
                }
            }
            if self.take_if_eq(b',').is_none() {
                break;
            }
        }

        Ok(auth::Challenge::Digest(digest))
    }

    fn parse_digest_credential(&mut self) -> Result<auth::Credential> {
        let mut digest = auth::DigestCredential::default();

        loop {
            self.skip_ws();
            let (name, value) = self.param_ref()?;

            match name {
                auth::REALM => digest.realm = value.map(String::from),
                auth::USERNAME => digest.username = value.map(String::from),
                auth::NONCE => digest.nonce = value.map(String::from),
                auth::URI => digest.uri = value.map(String::from),
                auth::RESPONSE => digest.response = value.map(String::from),
                auth::ALGORITHM => digest.algorithm = value.map(String::from),
                auth::CNONCE => digest.cnonce = value.map(String::from),
                auth::OPAQUE => digest.opaque = value.map(String::from),
                auth::QOP => digest.qop = value.map(String::from),
                auth::NC => digest.nc = value.map(String::from),
                _ => (), // Ignore unknown parameters
            }

            if self.take_if_eq(b',').is_none() {
                break;
            }
        }

        Ok(auth::Credential::Digest(digest))
    }

    fn parse_other_credential(&mut self, scheme: &'buf str) -> Result<auth::Credential> {
        let param = macros::parse_params!(self);

        Ok(auth::Credential::Other {
            scheme: scheme.to_owned(),
            param,
        })
    }

    pub fn parse_auth_credential(&mut self) -> Result<auth::Credential> {
        let scheme = self.token()?;

        if scheme == auth::DIGEST {
            return self.parse_digest_credential();
        }

        self.parse_other_credential(scheme)
    }

    #[inline]
    pub fn peek(&self) -> Option<&u8> {
        self.scanner.peek()
    }

    pub fn consume_while(&mut self, predicate: impl Fn(u8) -> bool) -> &'buf [u8] {
        self.scanner.scan_while(predicate)
    }

    pub fn read_line(&mut self) -> Result<&'buf str> {
        Ok(self.scanner.scan_line().map_err(ParseError::from)?)
    }

    #[inline]
    pub fn advance(&mut self) -> Result<u8> {
        Ok(self.scanner.next_byte().map_err(ParseError::from)?)
    }

    #[inline]
    pub fn parse_u32(&mut self) -> Result<u32> {
        Ok(self.scanner.scan_u32().map_err(ParseError::from)?)
    }

    #[inline]
    pub fn must_read(&mut self, scanner: u8) -> Result<()> {
        Ok(self.scanner.must_read(scanner).map_err(ParseError::from)?)
    }

    #[inline]
    pub fn parse_f32(&mut self) -> Result<f32> {
        Ok(self.scanner.scan_f32().map_err(ParseError::from)?)
    }

    #[inline]
    pub fn skip_ws(&mut self) {
        let _ = self.scanner.scan_while(scanner::is_space);
    }

    #[inline]
    pub fn skip_newline(&mut self) {
        let _ = self.scanner.scan_while(scanner::is_newline);
    }

    #[inline]
    pub fn take_alphabetic(&mut self) -> &'buf [u8] {
        self.scanner.scan_while(|b| b.is_ascii_alphabetic())
    }

    #[inline]
    pub fn take_until(&mut self, b: u8) -> &'buf [u8] {
        self.scanner.scan_until(b)
    }

    #[inline]
    pub fn take_if_eq(&mut self, b: u8) -> Option<u8> {
        self.scanner.scan_if_eq(b)
    }

    #[inline]
    pub fn is_next_newline(&self) -> bool {
        self.scanner.peek_if(scanner::is_newline).is_some()
    }

    #[inline]
    fn read_user(&mut self) -> &'buf str {
        unsafe { self.scanner.scan_while_as_str_unchecked(is_user) }
    }

    #[inline]
    fn read_pass(&mut self) -> &'buf str {
        unsafe { self.scanner.scan_while_as_str_unchecked(is_pass) }
    }

    #[inline]
    pub fn read_host(&mut self) -> &'buf str {
        unsafe { self.scanner.scan_while_as_str_unchecked(is_host) }
    }

    #[inline]
    pub fn read_token(&mut self) -> &'buf str {
        unsafe { self.scanner.scan_while_as_str_unchecked(is_token) }
    }

    #[inline]
    pub unsafe fn read_while_as_str_unchecked(&mut self, func: impl Fn(u8) -> bool) -> &'buf str {
        unsafe { self.scanner.scan_while_as_str_unchecked(func) }
    }

    pub unsafe fn param_ref_unchecked(
        &mut self,
        func: impl Fn(u8) -> bool,
    ) -> Result<(&'buf str, Option<&'buf str>)> {
        self.skip_ws();
        let name = unsafe { self.scanner.scan_while_as_str_unchecked(&func) };
        if self.take_if_eq(b'=').is_none() {
            return Ok((name, None));
        };
        let value = if self.take_if_eq(b'"').is_some() {
            // TODO: skip ignore \"\"
            let value = self.scanner.scan_while(|b| b != b'"');
            self.advance()?;
            str::from_utf8(value)?
        } else {
            unsafe { self.scanner.scan_while_as_str_unchecked(func) }
        };

        Ok((name, Some(value)))
    }

    pub fn parse_param(&mut self) -> Result<params::Param> {
        Ok(self.param_ref()?.into())
    }

    pub fn param_ref(&mut self) -> Result<params::ParamRef<'buf>> {
        unsafe { self.param_ref_unchecked(is_token) }
    }

    #[inline]
    fn parse_uri_header(&mut self) -> Result<params::Param> {
        Ok(unsafe { self.param_ref_unchecked(is_hdr_uri)?.into() })
    }

    fn parse_uri_param(&mut self) -> Result<params::ParamRef<'buf>> {
        unsafe { self.param_ref_unchecked(is_param) }
    }

    #[inline]
    pub fn via_param(&mut self) -> Result<params::ParamRef<'buf>> {
        unsafe { self.param_ref_unchecked(is_via_param) }
    }
}

#[must_use]
fn is_user(b: u8) -> bool {
    USER_TAB[b as usize]
}

#[must_use]
fn is_pass(b: u8) -> bool {
    PASS_TAB[b as usize]
}

#[must_use]
fn is_param(b: u8) -> bool {
    PARAM_TAB[b as usize]
}

#[must_use]
fn is_hdr_uri(b: u8) -> bool {
    HDR_TAB[b as usize]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Result;

    macro_rules! uri_test_ok {
        (name: $name:ident, input: $input:literal, expected: $expected:expr) => {
            #[test]
            fn $name() -> Result<()> {
                let uri = $crate::parser::SipParser::new($input).parse_sip_uri(true)?;

                assert_eq!($expected.scheme, uri.scheme());
                assert_eq!($expected.host_port.host, uri.host_port().host);
                assert_eq!($expected.host_port.port, uri.host_port().port);
                assert_eq!($expected.user, uri.user().cloned());
                assert_eq!($expected.transport_param, uri.transport_param());
                assert_eq!($expected.ttl_param, uri.ttl_param());
                assert_eq!($expected.method_param, uri.method_param());
                assert_eq!($expected.user_param.as_deref(), uri.user_param());
                assert_eq!($expected.lr_param, uri.lr_param());
                assert_eq!(&$expected.maddr_param, uri.maddr_param());

                for param in $expected.params.iter() {
                    assert_eq!($expected.params.param(&param.name), param.value.as_deref());
                }
                if let Some(headers) = uri.headers() {
                    assert!($expected.headers.is_some(), "missing headers!");
                    for param in $expected.headers.unwrap().iter() {
                        assert_eq!(headers.param(&param.name), param.value.as_deref());
                    }
                }

                Ok(())
            }
        };
    }

    uri_test_ok! {
        name: uri_test_1,
        input: "sip:biloxi.com",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sip)
            .host("biloxi.com".parse().unwrap())
            .build()
    }

    uri_test_ok! {
        name: uri_test_2,
        input: "sip:biloxi.com:5060",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sip)
            .host("biloxi.com:5060".parse().unwrap())
            .build()
    }

    uri_test_ok! {
        name: uri_test_3,
        input: "sip:a@b:5060",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sip)
            .user(uri::UserInfo { user: "a".to_owned(), pass: None})
            .host("b:5060".parse().unwrap())
            .build()
    }

    uri_test_ok! {
        name: uri_test_4,
        input: "sip:bob@biloxi.com:5060",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sip)
            .user(uri::UserInfo { user: "bob".to_owned(), pass: None})
            .host("biloxi.com:5060".parse().unwrap())
            .build()
    }

    uri_test_ok! {
        name: uri_test_5,
        input: "sip:bob@192.0.2.201:5060",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sip)
            .user(uri::UserInfo { user: "bob".to_owned(), pass: None})
            .host("192.0.2.201:5060".parse().unwrap())
            .build()
    }

    uri_test_ok! {
        name: uri_test_6,
        input: "sip:bob@[::1]:5060",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sip)
            .user(uri::UserInfo { user: "bob".to_owned(), pass: None})
            .host("[::1]:5060".parse().unwrap())
            .build()
    }

    uri_test_ok! {
        name: uri_test_7,
        input: "sip:bob:secret@biloxi.com",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sip)
            .user(uri::UserInfo { user: "bob".to_owned(), pass: Some("secret".to_owned())})
            .host("biloxi.com".parse().unwrap())
            .build()
    }

    uri_test_ok! {
        name: uri_test_8,
        input: "sip:bob:pass@192.0.2.201",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sip)
            .user(uri::UserInfo { user: "bob".to_owned(), pass: Some("pass".to_owned())})
            .host("192.0.2.201".parse().unwrap())
            .build()
    }

    uri_test_ok! {
        name: uri_test_9,
        input: "sip:bob@biloxi.com;foo=bar",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sip)
            .user(uri::UserInfo { user: "bob".to_owned(), pass: None})
            .host("biloxi.com".parse().unwrap())
            .param("foo".to_owned(), Some("bar".to_owned()))
            .build()
    }

    uri_test_ok! {
        name: uri_test_10,
        input: "sip:bob@biloxi.com:5060;foo=bar",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sip)
            .user(uri::UserInfo { user: "bob".to_owned(), pass: None})
            .host("biloxi.com:5060".parse().unwrap())
            .param("foo".to_owned(), Some("bar".to_owned()))
            .build()
    }

    uri_test_ok! {
        name: uri_test_11,
        input: "sips:bob@biloxi.com:5060",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sips)
            .user(uri::UserInfo { user: "bob".to_owned(), pass: None})
            .host("biloxi.com:5060".parse().unwrap())
            .build()
    }

    uri_test_ok! {
        name: uri_test_12,
        input: "sips:bob:pass@biloxi.com:5060",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sips)
            .user(uri::UserInfo { user: "bob".to_owned(), pass: Some("pass".to_owned()) })
            .host("biloxi.com:5060".parse().unwrap())
            .build()
    }

    uri_test_ok! {
        name: test_uri_11,
        input: "sip:bob@biloxi.com:5060;foo",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sip)
            .user(uri::UserInfo { user: "bob".to_owned(), pass: None})
            .param("foo".to_owned(), None)
            .host("biloxi.com:5060".parse().unwrap())
            .build()
    }

    uri_test_ok! {
        name: test_uri_12,
        input: "sip:bob@biloxi.com:5060;foo;baz=bar",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sip)
            .user(uri::UserInfo { user: "bob".to_owned(), pass: None})
            .host("biloxi.com:5060".parse().unwrap())
            .param("baz".to_owned(), Some("bar".to_owned()))
            .build()
    }

    uri_test_ok! {
        name: test_uri_13,
        input: "sip:bob@biloxi.com:5060;baz=bar;foo",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sip)
            .user(uri::UserInfo { user: "bob".to_owned(), pass: None})
            .host("biloxi.com:5060".parse().unwrap())
            .param("baz".to_owned(), Some("bar".to_owned()))
            .build()
    }

    uri_test_ok! {
        name: test_uri_14,
        input: "sip:bob@biloxi.com:5060;baz=bar;foo;a=b",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sip)
            .user(uri::UserInfo { user: "bob".to_owned(), pass: None})
            .host("biloxi.com:5060".parse().unwrap())
            .param("baz".to_owned(), Some("bar".to_owned()))
            .param("foo".to_owned(), None)
            .param("a".to_owned(), Some("b".to_owned()))
            .build()
    }

    uri_test_ok! {
        name: test_uri_15,
        input: "sip:bob@biloxi.com?foo=bar",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sip)
            .user(uri::UserInfo { user: "bob".to_owned(), pass: None})
            .host("biloxi.com".parse().unwrap())
            .header("foo".to_owned(), Some("bar".to_owned()))
            .build()
    }

    uri_test_ok! {
        name: test_uri_16,
        input: "sip:bob@biloxi.com?foo",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sip)
            .user(uri::UserInfo { user: "bob".to_owned(), pass: None})
            .host("biloxi.com".parse().unwrap())
            .header("foo".to_owned(), None)
            .build()
    }

    uri_test_ok! {
        name: test_uri_17,
        input: "sip:bob@biloxi.com:5060?foo=bar",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sip)
            .user(uri::UserInfo { user: "bob".to_owned(), pass: None})
            .host("biloxi.com:5060".parse().unwrap())
            .header("foo".to_owned(), Some("bar".to_owned()))
            .build()
    }

    uri_test_ok! {
        name: test_uri_18,
        input: "sip:bob@biloxi.com:5060?baz=bar&foo=&a=b",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sip)
            .user(uri::UserInfo { user: "bob".to_owned(), pass: None})
            .host("biloxi.com:5060".parse().unwrap())
            .header("baz".to_owned(), Some("bar".to_owned()))
            .header("foo".to_owned(), Some("".to_owned()))
            .header("a".to_owned(), Some("b".to_owned()))
            .build()
    }

    uri_test_ok! {
        name: test_uri_19,
        input: "sip:bob@biloxi.com:5060?foo=bar&baz",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sip)
            .user(uri::UserInfo { user: "bob".to_owned(), pass: None})
            .host("biloxi.com:5060".parse().unwrap())
            .header("foo".to_owned(), Some("bar".to_owned()))
            .header("baz".to_owned(), None)
            .build()
    }

    uri_test_ok! {
        name: test_uri_20,
        input: "sip:bob@biloxi.com;foo?foo=bar",
        expected: uri::Uri::builder()
            .scheme(uri::Scheme::Sip)
            .user(uri::UserInfo { user: "bob".to_owned(), pass: None})
            .host("biloxi.com".parse().unwrap())
            .param("foo".to_owned(), None)
            .header("foo".to_owned(), Some("bar".to_owned()))
            .build()
    }
}
