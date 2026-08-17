//! Transport layer.
//!
//! This module defines different transport protocols used to
//! exchange SIP messages between entities.
//!
//! All transports implement the [`SipTransport`] trait and expose a
//! common interface for sending and receiving SIP messages.
//!
//! The [`Transport`] struct is a wrapper around any transport implementation
//! that allows for easy management of different transport types.
//!
//! # Available TransportLayer
//!
//! - [`udp`]: SIP over UDP transport implementation.
//! - [`tcp`]: SIP over TCP transport implementation.
//! - [`ws`]:  SIP over WebSocket transport implementation.

use std::net::{IpAddr, SocketAddr};
use std::result::Result as StdResult;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use std::{fmt, io, ops};

use async_trait::async_trait;
use bytes::Bytes;

use self::incoming::{IncomingInfo, IncomingRequest, IncomingResponse, MandatoryHeaders};
use self::tcp::TcpTransport;
use self::ws::WebSocketTransport;
use crate::endpoint::{Endpoint, WeakEndpointHandle};
use crate::error::{Error, Result};
#[cfg(test)]
use crate::message::Request;
use crate::message::SipMessage;
use crate::message::uri::{HostPort, Scheme, Uri};
use crate::parser::SipParser;
use crate::resolver::{DefaultResolver, DomainResolver, ServerAddresses, SipHost};

// Core Transport modules
mod decode;

pub mod incoming;
pub mod outgoing;
pub mod tcp;
pub mod udp;
pub mod ws;

/// Keep-alive Request.
pub const KEEPALIVE_REQUEST: &[u8] = b"\r\n\r\n";

/// Keep-alive Response.
pub const KEEPALIVE_RESPONSE: &[u8] = b"\r\n";

/// Marks the end of headers in a SIP message.
pub const MSG_HEADERS_END: &[u8] = b"\r\n\r\n";

#[derive(Clone)]
pub struct TransportLayer {
    endpoint: WeakEndpointHandle,
    resolver: DomainResolver,
    transports: Arc<Mutex<rustc_hash::FxHashMap<TransportKey, TransportHandle>>>,
}

/// A wrapper around a SIP transport implementation.
#[derive(Clone)]
pub struct TransportHandle {
    /// Shared transport instance.
    shared: Arc<dyn SipTransport>,
}

/// Trait for all SIP transport implementations.
#[async_trait]
pub trait SipTransport: Send + Sync + 'static {
    /// Sends data on the socket to the given address. On success, returns the
    /// number of bytes written.
    async fn send_msg(&self, buf: &[u8], address: &SocketAddr) -> Result<usize>;

    /// Get transport type.
    fn protocol(&self) -> TransportProtocol;

    /// Get the local socket address addr to this transport.
    fn local_addr(&self) -> SocketAddr;

    /// Get the remote socket address addr to this transport (if any).
    fn remote_addr(&self) -> Option<SocketAddr>;

    /// Returns `true` if the transport is reliable.
    fn is_reliable(&self) -> bool;

    /// Returns `true` if the transport is unreliable.
    fn is_unreliable(&self) -> bool {
        !self.is_reliable()
    }

    /// Returns `true` if the transport is secure.
    fn is_secure(&self) -> bool;

    /// Get the key that uniquely identifies this transport.
    fn key(&self) -> TransportKey {
        TransportKey::from(self)
    }
}

/// Unique key for a transport instance.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct TransportKey {
    /// The destination address of the transport.
    pub socket_addr: SocketAddr,
    /// The transport type (e.g., UDP, TCP, TLS).
    pub protocol: TransportProtocol,
}

/// Represents the type of transport.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportProtocol {
    /// Udp.
    #[default]
    Udp,
    /// Tcp.
    Tcp,
    /// WebSocket.
    Ws,
    /// Websocket with tls.
    Wss,
    /// Tcp with tls
    Tls,
    /// Sctp.
    Sctp,
}

/// A raw network packet.
#[derive(Clone)]
pub struct Packet {
    /// Raw packet payload.
    pub data: Bytes,
    /// Remote address of the sender.
    pub source: SocketAddr,
    /// Time when the packet was received.
    pub time: SystemTime,
}

/// A network packet received through a transport.
#[derive(Clone)]
pub struct TransportMessage {
    /// Transport that received the packet.
    pub transport: TransportHandle,
    /// The raw packet data and metadata.
    pub packet: Packet,
}

impl TransportLayer {
    pub fn new(endpoint: WeakEndpointHandle) -> Self {
        Self {
            endpoint,
            transports: Default::default(),
            resolver: DomainResolver::from(DefaultResolver),
        }
    }

    /// Add a new transport to the transports.
    pub fn register_transport(&self, key: TransportKey, value: TransportHandle) {
        self.insert(key, value);
    }

    /// Remove a transport by its key.
    pub fn remove_transport(&self, key: &TransportKey) {
        self.remove(key);
    }

    pub fn get_transport(&self, key: &TransportKey) -> Option<TransportHandle> {
        self.get(key)
    }

    pub async fn select_transport(
        &self,
        socket_addr: SocketAddr,
        protocol: TransportProtocol,
    ) -> Result<TransportHandle> {
        let key = TransportKey {
            socket_addr,
            protocol,
        };
        if let Some(transport) = self.get_transport(&key) {
            return Ok(transport);
        }
        if let TransportProtocol::Tcp = protocol {
            TcpTransport::connect(socket_addr, self).await
        } else if let TransportProtocol::Ws | TransportProtocol::Wss = protocol {
            let url = if protocol == TransportProtocol::Ws {
                format!("ws://{socket_addr}")
            } else {
                format!("wss://{socket_addr}")
            };
            let timeout = Duration::from_secs(1);
            WebSocketTransport::connect(url, timeout, self).await
        } else {
            Err(Error::UnsupportedTransport)
        }
    }

    // RFC 3263 Section 4
    // RFC 3263 Section 4.1
    // RFC 3263 Section 4.2
    pub async fn resolve_uri(&self, uri: &Uri) -> io::Result<ServerAddresses> {
        let host = uri.maddr_param.as_ref().unwrap_or(&uri.host_port.host);
        let transport = match uri.transport_param {
            Some(transport) => transport,
            None => TransportProtocol::from_scheme(uri.scheme),
        };
        let port = uri.host_port.port;
        let host_port = HostPort {
            host: host.to_owned(),
            port,
        };
        let sip_host = SipHost {
            host_port,
            protocol: Some(transport),
        };

        self.resolver.resolve(&sip_host).await
    }

    pub(self) fn receive_message(&self, message: TransportMessage) {
        let Some(endpoint) = self.endpoint.upgrade() else {
            return;
        };
        tokio::spawn(Self::process_message(endpoint, message));
    }

    pub(self) async fn process_message(endpoint: Endpoint, message: TransportMessage) {
        let TransportMessage { transport, packet } = &message;
        let sip_message = match SipParser::parse(&packet.data) {
            Ok(parsed) => parsed,
            Err(err) => {
                log::warn!(
                    "Ignoring {} bytes packet from {} {} : {}\n{}-- end of packet.",
                    packet.data.len(),
                    transport.protocol().as_upper_case(),
                    packet.source,
                    err,
                    String::from_utf8_lossy(&packet.data)
                );
                return;
            }
        };

        let mut mandatory_headers = match MandatoryHeaders::try_from(sip_message.headers()) {
            Ok(headers) => headers,
            Err(err) => {
                log::error!("{err}");
                return;
            }
        };

        mandatory_headers.via.received = Some(packet.source.ip());

        let info = IncomingInfo {
            mandatory_headers,
            transport_msg: message,
        };

        let incoming_info = Box::new(info);

        match sip_message {
            SipMessage::Request(request) => {
                endpoint
                    .on_request(IncomingRequest {
                        request,
                        incoming_info,
                    })
                    .await;
            }
            SipMessage::Response(response) => {
                endpoint
                    .on_response(IncomingResponse {
                        response,
                        incoming_info,
                    })
                    .await;
            }
        }
    }

    pub fn resolver(&self) -> &DomainResolver {
        &self.resolver
    }

    fn insert(&self, key: TransportKey, value: TransportHandle) {
        let mut map = self.transports.lock().expect("Lock failed");

        map.insert(key, value);
    }

    /// Remove a transport by its key.
    fn remove(&self, key: &TransportKey) {
        let mut map = self.transports.lock().expect("Lock failed");

        map.remove(key);
    }

    fn get(&self, key: &TransportKey) -> Option<TransportHandle> {
        let map = self.transports.lock().expect("Lock failed");

        if let Some(transport) = map.get(key) {
            return Some(transport.clone());
        }

        if key.protocol.is_unreliable() {
            let target_ip = key.socket_addr.ip();
            let target_proto = key.protocol;

            let existing = map.values().find(|transport| {
                let ip = transport.local_addr().ip();

                transport.protocol() == target_proto
                    && matches!(
                        (ip, target_ip),
                        (IpAddr::V4(_), IpAddr::V4(_)) | (IpAddr::V6(_), IpAddr::V6(_))
                    )
            });

            if let Some(transport) = existing {
                return Some(transport.clone());
            }
        }

        None
    }
}

impl Default for TransportLayer {
    fn default() -> Self {
        Self {
            endpoint: Default::default(),
            resolver: DomainResolver::from(DefaultResolver),
            transports: Default::default(),
        }
    }
}

impl TransportHandle {
    /// Creates a new `Transport` instance with the given implementation.
    pub fn new(transport: impl SipTransport) -> Self {
        Self {
            shared: Arc::new(transport),
        }
    }
}

impl ops::Deref for TransportHandle {
    type Target = dyn SipTransport;

    fn deref(&self) -> &Self::Target {
        &*self.shared
    }
}

impl TransportProtocol {
    pub fn is_reliable(self) -> bool {
        matches!(
            self,
            Self::Tcp | Self::Tls | Self::Sctp | Self::Wss | Self::Ws
        )
    }

    pub fn is_unreliable(self) -> bool {
        !self.is_reliable()
    }

    pub const fn as_lower_case(self) -> &'static str {
        match self {
            Self::Udp => "udp",
            Self::Tcp => "tcp",
            Self::Ws => "ws",
            Self::Wss => "wss",
            Self::Tls => "tls",
            Self::Sctp => "sctp",
        }
    }

    pub const fn as_upper_case(self) -> &'static str {
        match self {
            Self::Udp => "UDP",
            Self::Tcp => "TCP",
            Self::Tls => "TLS",
            Self::Sctp => "SCTP",
            Self::Ws => "WS",
            Self::Wss => "WSS",
        }
    }

    pub(crate) fn from_scheme(scheme: Scheme) -> Self {
        match scheme {
            Scheme::Sip => Self::Udp,
            Scheme::Sips => Self::Tcp,
        }
    }

    /// Returns true if the transport is secure.
    pub fn is_secure(self) -> bool {
        matches!(self, Self::Tls | Self::Wss)
    }

    /// Returns the default port number associated with the transport.
    #[inline]
    pub const fn default_port(&self) -> u16 {
        match self {
            Self::Udp | Self::Tcp | Self::Sctp => 5060,
            Self::Tls => 5061,
            Self::Ws | Self::Wss => 80,
        }
    }
}

impl FromStr for TransportProtocol {
    type Err = ();

    fn from_str(s: &str) -> StdResult<Self, Self::Err> {
        match s {
            s if s.eq_ignore_ascii_case("udp") => Ok(Self::Udp),
            s if s.eq_ignore_ascii_case("tcp") => Ok(Self::Tcp),
            s if s.eq_ignore_ascii_case("ws") => Ok(Self::Ws),
            s if s.eq_ignore_ascii_case("wss") => Ok(Self::Wss),
            s if s.eq_ignore_ascii_case("tls") => Ok(Self::Tls),
            s if s.eq_ignore_ascii_case("sctp") => Ok(Self::Sctp),
            _ => Err(()),
        }
    }
}

impl TryFrom<&str> for TransportProtocol {
    type Error = ();

    fn try_from(s: &str) -> StdResult<Self, Self::Error> {
        Self::from_str(s)
    }
}

impl<T> From<&T> for TransportKey
where
    T: SipTransport + ?Sized,
{
    fn from(transport: &T) -> Self {
        let socket_addr = transport.local_addr();
        let protocol = transport.protocol();

        Self {
            socket_addr,
            protocol,
        }
    }
}

impl Packet {
    /// Creates a new `Packet` whith the given `data` and `source` addr.
    pub fn new(data: Bytes, source: SocketAddr) -> Self {
        Self {
            data,
            source,
            time: SystemTime::now(),
        }
    }
}

/// A mock transport, for testing purposes
#[derive(Clone)]
#[cfg(test)]
pub(crate) struct MockTransport {
    sent: Arc<Mutex<Vec<(Vec<u8>, SocketAddr)>>>,
    addr: SocketAddr,
    protocol: TransportProtocol,
    fail_at: Option<usize>,
}

#[cfg(test)]
impl MockTransport {
    pub fn with_protocol(protocol: TransportProtocol) -> Self {
        use std::net::Ipv4Addr;

        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let port = protocol.default_port();
        let mock = Self {
            sent: Default::default(),
            addr: SocketAddr::new(ip, port),
            protocol,
            fail_at: None,
        };

        mock
    }

    pub fn new_udp() -> Self {
        Self::with_protocol(TransportProtocol::Udp)
    }

    pub fn new_tcp() -> Self {
        Self::with_protocol(TransportProtocol::Tcp)
    }

    pub fn new_tls() -> Self {
        Self::with_protocol(TransportProtocol::Tls)
    }

    pub fn last_request(&self) -> Option<Request> {
        self.last_message().map(|msg| {
            if let SipMessage::Request(req) = msg {
                Some(req)
            } else {
                None
            }
        })?
    }

    pub fn sent_count(&self) -> usize {
        self.sent.lock().unwrap().len()
    }

    pub fn last_message(&self) -> Option<SipMessage> {
        let guard = self.sent.lock().unwrap();
        guard
            .last()
            .map(|(buff, _)| buff)
            .cloned()
            .map(|b| SipParser::parse(&b).unwrap())
    }

    fn push_msg(&self, (buf_vec, address): (Vec<u8>, SocketAddr)) -> usize {
        let mut guard = self.sent.lock().unwrap();
        guard.push((buf_vec, address));
        guard.len()
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl SipTransport for MockTransport {
    async fn send_msg(&self, buf: &[u8], address: &SocketAddr) -> crate::Result<usize> {
        let current_count = self.push_msg((buf.to_vec(), *address));

        if let Some(fail_at) = self.fail_at
            && fail_at == current_count
        {
            return Err(crate::Error::Transport("Simulated failure".into()));
        }

        Ok(buf.len())
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        None
    }

    fn protocol(&self) -> TransportProtocol {
        self.protocol
    }

    fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    fn is_reliable(&self) -> bool {
        self.protocol.is_reliable()
    }

    fn is_secure(&self) -> bool {
        self.protocol.is_secure()
    }
}
