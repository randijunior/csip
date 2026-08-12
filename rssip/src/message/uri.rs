use std::{borrow, fmt, net, ops, str};

use crate::error;
use crate::message::method::SipMethod;
use crate::message::params::{Param, Params};
use crate::parser::SipParser;
use crate::transport::TransportProtocol;

type Port = u16;

/// A SIP uri.
///
/// This enum can contain either an [`Uri`] or an [`NameAddr`].
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum SipUri {
    /// A plain SIP URI.
    Uri(Uri),
    /// A named address.
    NameAddr(NameAddr),
}

/// Represents an SIP `name-addr`.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct NameAddr {
    /// The optional display part.
    pub display: Option<DisplayName>,
    /// The uri of the `name-addr`.
    pub uri: Uri,
}

/// Represents an display name in `NameAddr`
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct DisplayName(String);

/// An plain SIP uri.
#[derive(Debug, PartialEq, Eq, Default, Clone)]
pub struct Uri {
    /// The uri scheme.
    pub scheme: Scheme,
    /// Optional user part of uri.
    pub user: Option<UserInfo>,
    /// The uri host.
    pub host_port: HostPort,
    /// The user parameter.
    pub user_param: Option<String>,
    /// The method parameter.
    pub method_param: Option<SipMethod>,
    /// The transport parameter.
    pub transport_param: Option<TransportProtocol>,
    /// The ttl parameter.
    pub ttl_param: Option<u8>,
    /// The lr parameter.
    pub lr_param: bool,
    /// The maddr parameter.
    pub maddr_param: Option<Host>,
    /// Other parameters.
    pub params: Params,
    /// Optional header parameters
    pub headers: Option<UriHeaders>,
}

/// A SIP URI scheme, either `sip` or `sips`.
#[derive(Debug, PartialEq, Eq, Clone, Default, Copy)]
pub enum Scheme {
    /// An Sip uri scheme.
    #[default]
    Sip,
    /// An Sips uri scheme.
    Sips,
}

/// Represents the user information component of a URI.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct UserInfo {
    /// The username part of the URI.
    pub user: String,
    /// The optional password associated with the user.
    pub pass: Option<String>,
}

/// Represents a combination of a host (domain or IP address) and an optional
/// port.
#[derive(Debug, Default, PartialEq, Eq, Clone, Hash)]
pub struct HostPort {
    /// The host part, which may be a domain name or an IP address.
    pub host: Host,
    /// The optional port number.
    pub port: Option<Port>,
}

/// Represents the host part of a URI, which can be either a
/// domain name or an IP address.
#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub enum Host {
    /// A domain name, such as `example.com`.
    HostName(HostName),
    /// An IP address, either IPv4 or IPv6.
    IpAddr(net::IpAddr),
}

/// Represents a domain name in a SIP URI.
#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub struct HostName(borrow::Cow<'static, str>);

/// Represents the header parameters of a SIP URI.
#[derive(Debug, PartialEq, Eq, Default, Clone)]
pub struct UriHeaders(Params);

impl SipUri {
    /// Returns a reference to the contained [`Uri`] value.
    pub fn uri(&self) -> &Uri {
        match self {
            SipUri::Uri(uri) => uri,
            SipUri::NameAddr(name_addr) => &name_addr.uri,
        }
    }

    /// Returns a reference to the contained [`NameAddr`] if this is
    /// a `name-addr` variant.
    pub fn name_addr(&self) -> Option<&NameAddr> {
        if let SipUri::NameAddr(addr) = self {
            Some(addr)
        } else {
            None
        }
    }

    /// Returns a reference to the display part if present.
    pub fn display(&self) -> Option<&DisplayName> {
        if let SipUri::NameAddr(addr) = self {
            addr.display()
        } else {
            None
        }
    }

    /// Returns the scheme of the uri.
    pub fn scheme(&self) -> Scheme {
        match self {
            SipUri::Uri(uri) => uri.scheme,
            SipUri::NameAddr(addr) => addr.uri.scheme,
        }
    }

    /// Returns the user part of the uri.
    pub fn user(&self) -> Option<&UserInfo> {
        match self {
            SipUri::Uri(uri) => uri.user.as_ref(),
            SipUri::NameAddr(addr) => addr.uri.user.as_ref(),
        }
    }

    /// Returns a reference to the [`HostPort`] of the uri.
    pub fn host_port(&self) -> &HostPort {
        match self {
            SipUri::Uri(uri) => &uri.host_port,
            SipUri::NameAddr(addr) => &addr.uri.host_port,
        }
    }

    /// Returns the `transport` parameter.
    pub fn transport_param(&self) -> Option<TransportProtocol> {
        match self {
            SipUri::Uri(uri) => uri.transport_param,
            SipUri::NameAddr(addr) => addr.uri.transport_param,
        }
    }

    /// Returns the user parameter of the uri.
    pub fn user_param(&self) -> Option<&str> {
        match self {
            SipUri::Uri(uri) => uri.user_param.as_deref(),
            SipUri::NameAddr(addr) => addr.uri.user_param.as_deref(),
        }
    }

    /// Returns the method parameter of the uri.
    pub fn method_param(&self) -> Option<SipMethod> {
        match self {
            SipUri::Uri(uri) => uri.method_param,
            SipUri::NameAddr(addr) => addr.uri.method_param,
        }
    }

    /// Returns the ttl parameter of the uri.
    pub fn ttl_param(&self) -> Option<u8> {
        match self {
            SipUri::Uri(uri) => uri.ttl_param,
            SipUri::NameAddr(addr) => addr.uri.ttl_param,
        }
    }

    /// Returns the lr parameter of the uri.
    pub fn lr_param(&self) -> bool {
        match self {
            SipUri::Uri(uri) => uri.lr_param,
            SipUri::NameAddr(addr) => addr.uri.lr_param,
        }
    }

    /// Returns the maddr parameter of the uri.
    pub fn maddr_param(&self) -> &Option<Host> {
        match self {
            SipUri::Uri(uri) => &uri.maddr_param,
            SipUri::NameAddr(addr) => &addr.uri.maddr_param,
        }
    }

    /// Returns the other parameters of the uri.
    pub fn other_params(&self) -> &Params {
        match self {
            SipUri::Uri(uri) => &uri.params,
            SipUri::NameAddr(addr) => &addr.uri.params,
        }
    }

    /// Returns the header parameters of the uri.
    pub fn headers(&self) -> Option<&UriHeaders> {
        match self {
            SipUri::Uri(uri) => uri.headers.as_ref(),
            SipUri::NameAddr(addr) => addr.uri.headers.as_ref(),
        }
    }
}

impl str::FromStr for SipUri {
    type Err = error::Error;

    fn from_str(s: &str) -> error::Result<Self> {
        SipParser::new(s).parse_sip_uri(true)
    }
}

impl fmt::Display for SipUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SipUri::Uri(uri) => write!(f, "{}", uri),
            SipUri::NameAddr(addr) => write!(f, "{}", addr),
        }
    }
}

impl Uri {
    /// Creates a new builder to create an `Uri`.
    pub fn builder() -> UriBuilder {
        UriBuilder::new()
    }
}

impl str::FromStr for Uri {
    type Err = error::Error;

    fn from_str(s: &str) -> error::Result<Self> {
        let mut p = SipParser::new(s);

        p.parse_uri(true)
    }
}

impl fmt::Display for Uri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.scheme)?;

        write!(f, ":")?;

        if let Some(user_info) = &self.user {
            write!(f, "{}", user_info.user)?;

            if let Some(pass) = &user_info.pass {
                write!(f, ":{}", pass)?;
            }

            write!(f, "@")?;
        }

        write!(f, "{}", self.host_port)?;

        if let Some(user) = &self.user_param {
            write!(f, ";user={}", user)?;
        }

        if let Some(method) = &self.method_param {
            write!(f, ";method={}", method)?;
        }

        if let Some(maddr) = &self.maddr_param {
            write!(f, ";maddr={}", maddr)?;
        }

        if let Some(transport) = &self.transport_param {
            write!(f, ";transport={}", transport.as_lower_case())?;
        }

        if let Some(ttl) = self.ttl_param {
            write!(f, ";ttl={}", ttl)?;
        }

        if self.lr_param {
            write!(f, ";lr")?;
        }

        write!(f, "{}", self.params)?;

        if let Some(headers) = &self.headers {
            write!(f, "?{}", headers)?;
        }

        Ok(())
    }
}

impl fmt::Display for Scheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scheme::Sip => write!(f, "sip"),
            Scheme::Sips => write!(f, "sips"),
        }
    }
}

/// Builder for creating a new SIP URI.
#[derive(Default)]
pub struct UriBuilder {
    uri: Uri,
}

impl UriBuilder {
    /// Returns a builder to create an `UriBuilder`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the uri scheme.
    pub fn scheme(mut self, scheme: Scheme) -> Self {
        self.uri.scheme = scheme;
        self
    }

    /// Sets the user part of the uri.
    pub fn user(mut self, user: UserInfo) -> Self {
        self.uri.user = Some(user);
        self
    }

    /// Sets the host of the uri.
    pub fn host(mut self, host_port: HostPort) -> Self {
        self.uri.host_port = host_port;
        self
    }

    /// Sets the user parameter of the uri.
    pub fn user_param(mut self, param: String) -> Self {
        self.uri.user_param = Some(param);
        self
    }

    /// Sets the method parameter of the uri.
    pub fn method_param(mut self, param: SipMethod) -> Self {
        self.uri.method_param = Some(param);
        self
    }

    /// Sets the transport parameter of the uri.
    pub fn transport_param(mut self, param: TransportProtocol) -> Self {
        self.uri.transport_param = Some(param);
        self
    }

    /// Sets the ttl parameter of the uri.
    pub fn ttl_param(mut self, param: u8) -> Self {
        self.uri.ttl_param = Some(param);
        self
    }

    /// Sets the lr parameter of the uri.
    pub fn lr_param(mut self, param: bool) -> Self {
        self.uri.lr_param = param;
        self
    }

    /// Sets the maddr parameter of the uri.
    pub fn maddr_param(mut self, param: Host) -> Self {
        self.uri.maddr_param = Some(param);
        self
    }

    /// Set generic parameter of the uri.
    pub fn param(mut self, name: String, value: Option<String>) -> Self {
        self.uri.params.push(Param { name, value });

        self
    }

    /// Set header parameter of the uri.
    pub fn header(mut self, name: String, value: Option<String>) -> Self {
        let headers = self.uri.headers.get_or_insert_default();

        headers.push(Param { name, value });

        self
    }

    pub fn headers(mut self, headers: UriHeaders) -> Self {
        self.uri.headers = Some(headers);

        self
    }

    /// Finalize the builder into a `Uri`.
    pub fn build(self) -> Uri {
        self.uri
    }
}

impl DisplayName {
    /// Creates a new `DisplayName` whith the given `display`.
    #[inline]
    pub fn new(display: String) -> Self {
        Self(display)
    }

    /// Returns the inner phrase as str.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl NameAddr {
    pub fn new(uri: Uri) -> Self {
        Self { display: None, uri }
    }

    /// Returns a reference to the display part if present.
    pub fn display(&self) -> Option<&DisplayName> {
        self.display.as_ref()
    }
}

impl str::FromStr for NameAddr {
    type Err = error::Error;

    fn from_str(s: &str) -> error::Result<Self> {
        SipParser::new(s).parse_name_addr()
    }
}

impl fmt::Display for NameAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(display) = &self.display {
            write!(f, "\"{}\" ", display.0)?;
        }
        write!(f, "<{}>", self.uri)?;

        Ok(())
    }
}

impl HostName {
    /// Creates a new `HostName`.
    pub fn new(name: borrow::Cow<'static, str>) -> Self {
        HostName(name)
    }

    /// Returns the string representation of the domain name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for HostName {
    fn from(name: &str) -> Self {
        Self::new(borrow::Cow::Owned(name.to_owned()))
    }
}

impl fmt::Display for HostName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<net::SocketAddr> for HostPort {
    fn from(socket_addr: net::SocketAddr) -> Self {
        Self {
            host: Host::IpAddr(socket_addr.ip()),
            port: Some(socket_addr.port()),
        }
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Host::HostName(domain) => write!(f, "{domain}"),
            Host::IpAddr(ip_addr) => write!(f, "{ip_addr}"),
        }
    }
}

impl Host {
    /// Returns the string representation of the host as a `borrow::Cow<str>`.
    ///
    /// If the host is a domain name, this returns a borrowed string. If the
    /// host is an IP address, this returns an owned string created via
    /// formatting.
    pub fn as_str(&self) -> borrow::Cow<'_, str> {
        match self {
            Host::HostName(host) => borrow::Cow::Borrowed(host.as_str()),
            Host::IpAddr(ip_addr) => borrow::Cow::Owned(ip_addr.to_string()),
        }
    }

    pub fn is_domain(&self) -> bool {
        matches!(self, Host::HostName(_))
    }

    pub fn is_ip_addr(&self) -> bool {
        !self.is_domain()
    }
}

impl str::FromStr for Host {
    type Err = error::Error;

    fn from_str(s: &str) -> error::Result<Self> {
        if let Ok(ip_addr) = s.parse::<net::IpAddr>() {
            Ok(Host::IpAddr(ip_addr))
        } else {
            Ok(Host::HostName(HostName::from(s)))
        }
    }
}

impl From<net::IpAddr> for Host {
    fn from(ip_addr: net::IpAddr) -> Self {
        Self::IpAddr(ip_addr)
    }
}

impl Default for Host {
    fn default() -> Self {
        Self::HostName(HostName::new("localhost".into()))
    }
}

impl HostPort {
    /// Returns the IP address if the host is an IP address, otherwise `None`.
    pub fn ip_addr(&self) -> Option<net::IpAddr> {
        if let Host::IpAddr(ip_addr) = self.host {
            Some(ip_addr)
        } else {
            None
        }
    }
}

impl str::FromStr for HostPort {
    type Err = error::Error;

    fn from_str(s: &str) -> error::Result<Self> {
        let mut p = SipParser::new(s);

        p.parse_host_port()
    }
}

impl fmt::Display for HostPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.host {
            Host::HostName(domain) => f.write_str(&domain.0)?,
            Host::IpAddr(ip_addr) => write!(f, "{}", ip_addr)?,
        }
        if let Some(port) = self.port {
            write!(f, ":{}", port)?;
        }
        Ok(())
    }
}

impl From<Host> for HostPort {
    fn from(host: Host) -> Self {
        Self { host, port: None }
    }
}

impl UriHeaders {
    pub fn new(params: Params) -> Self {
        Self(params)
    }
}

impl fmt::Display for UriHeaders {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let formater = itertools::Itertools::format_with(self.iter(), "&", |param, f| {
            f(&format_args!(
                "{}={}",
                param.name,
                param.value.as_ref().map_or("", |v| v.as_str())
            ))
        });
        write!(f, "{}", formater)?;
        Ok(())
    }
}

impl ops::Deref for UriHeaders {
    type Target = Params;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ops::DerefMut for UriHeaders {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
