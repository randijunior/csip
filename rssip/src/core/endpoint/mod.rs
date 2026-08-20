//! SIP Endpoint

mod builder;
mod plugin;

use std::any::type_name;
use std::borrow::Cow;
use std::net::SocketAddr;
use std::sync;
use std::sync::Arc;

pub use builder::EndpointBuilder;
use bytes::Bytes;
pub use plugin::{Plugin, ToTake};
use utils::encode::Encode;

use self::plugin::Plugins;
use super::resolver::{LookupAddress, SipHost};
use crate::Result;
use crate::error::Error;
use crate::message::headers::{Accept, Allow, Header, Headers, Route, Supported};
use crate::message::status_code::StatusCode;
use crate::message::uri::{Host, HostPort, NameAddr, Uri};
use crate::message::{ReasonPhrase, Request, Response, StatusLine};
use crate::sip_ua::dialog::DialogPlugin;
use crate::transaction::TsxPlugin;
use crate::transport::incoming::{IncomingRequest, IncomingResponse};
use crate::transport::outgoing::{
    OutgoingDestInfo, OutgoingRequest, OutgoingResponse, TargetTransportInfo,
};
use crate::transport::{TransportHandle, TransportLayer};

struct EndpointInner {
    /// The transport layer for the endpoint.
    transport: TransportLayer,
    /// The name of the endpoint.
    name: String,
    /// The capability header list.
    allow: Allow,
    accept: Accept,
    supported: Supported,
    /// The list of plugins registered.
    plugins: Plugins,
}

/// A SIP endpoint.
#[derive(Clone)]
pub struct Endpoint {
    inner: Arc<EndpointInner>,
}

/// A handle to endpoint internal
///
/// This contains a weak reference to the endpoint.
#[derive(Debug, Default, Clone)]
pub(crate) struct WeakEndpointHandle(sync::Weak<EndpointInner>);

impl Endpoint {
    pub fn builder() -> EndpointBuilder {
        EndpointBuilder::default()
    }

    /// Get the endpoint name.
    pub fn name(&self) -> &str {
        self.inner.name.as_str()
    }

    pub(crate) fn plugin<M: plugin::Plugin>(&self) -> &M {
        self.inner
            .plugins
            .find_plugin()
            .ok_or_else(|| format!("endpoint missing plugin {}", type_name::<M>()))
            .unwrap()
    }

    pub(crate) fn supported(&self) -> &Supported {
        &self.inner.supported
    }

    pub(crate) fn accept(&self) -> &Accept {
        &self.inner.accept
    }

    pub(crate) fn allow(&self) -> &Allow {
        &self.inner.allow
    }

    pub(crate) fn create_response(
        &self,
        request: &IncomingRequest,
        code: StatusCode,
        reason: Option<ReasonPhrase>,
    ) -> Response {
        let all_hdrs = &request.request.headers;
        let mandatory_headers = &request.incoming_info.mandatory_headers;

        // Copy the necessary headers from the request.
        let mut headers = Headers::with_capacity(7);

        // `Via` header.
        let topmost_via = mandatory_headers.via.clone();
        headers.push(Header::Via(topmost_via));
        let other_vias = all_hdrs
            .iter()
            .filter(|h| matches!(h, Header::Via(_)))
            .skip(1);
        headers.extend(other_vias.cloned());

        // `Record-Route` header.
        let rr = all_hdrs
            .iter()
            .filter(|h| matches!(h, Header::RecordRoute(_)));
        headers.extend(rr.cloned());

        // `Call-ID` header.
        headers.push(Header::CallId(mandatory_headers.call_id.clone()));

        // `From` header.
        headers.push(Header::From(mandatory_headers.from.clone()));

        // `To` header.
        let mut to = mandatory_headers.to.clone();
        // 8.2.6.2 Headers and Tags
        // The UAS MUST add a tag to the To header field in
        // the response (with the exception of the 100 (Trying)
        // response, in which a tag MAY be present).
        if to.tag().is_none() && code.as_u16() > 100 {
            to.set_tag(mandatory_headers.via.branch.as_ref().map(|b| b.to_owned()));
        }
        headers.push(Header::To(to));

        // `CSeq` header.
        headers.push(Header::CSeq(mandatory_headers.cseq));

        let reason = match reason {
            None => code.reason(),
            Some(reason) => reason,
        };
        let status_line = StatusLine { code, reason };

        Response {
            status_line,
            headers,
            body: None,
        }
    }

    /// Creates a new SIP response based on an incoming
    /// request.
    pub fn create_outgoing_response(
        &self,
        request: &IncomingRequest,
        code: StatusCode,
        reason: Option<ReasonPhrase>,
    ) -> OutgoingResponse {
        let response = self.create_response(request, code, reason);
        let dest_info = self.get_response_destination(request);

        OutgoingResponse {
            response,
            dest_info,
            encoded: Bytes::new(),
        }
    }

    /// Send the request.
    pub async fn send_outgoing_request(&self, request: &mut OutgoingRequest) -> Result<()> {
        if request.encoded.is_empty() {
            request.encoded = request.encode()?;
        }

        log::debug!(
            "Sending Request {} to /{}",
            request.request.req_line.method,
            request.target_info.socket_addr
        );

        for plugin in self.inner.plugins.plugins() {
            plugin.on_outgoing_request(request).await;
        }

        if let Err(err) = request
            .target_info
            .transport
            .send_msg(&request.encoded, &request.target_info.socket_addr)
            .await
        {
            log::error!("Failed to send request: {}", err);
        }

        Ok(())
    }

    pub async fn send_outgoing_response(&self, response: &mut OutgoingResponse) -> Result<()> {
        if response.encoded.is_empty() {
            response.encoded = response.encode()?;
        }
        log::debug!(
            "Sending Response {} {} to /{}",
            response.status_line.code.as_u16(),
            response.status_line.reason.as_str(),
            response.dest_info
        );

        for plugin in self.inner.plugins.plugins() {
            plugin.on_outgoing_response(response).await;
        }

        self.send_response(response).await?;

        Ok(())
    }

    // RFC 3263 - 5 Server Usage
    async fn send_response(&self, response: &mut OutgoingResponse) -> Result<()> {
        let target = &mut response.dest_info;

        if let Some(target_info) = &target.transport
            && target_info
                .transport
                .send_msg(&response.encoded, &target_info.socket_addr)
                .await
                .is_ok()
        {
            return Ok(());
        }

        let (host_port, proto) = target.host_port.clone();
        let domain = SipHost {
            host_port,
            protocol: Some(proto),
        };

        let addresses = self.transports().resolver().resolve(&domain).await?;

        for addr in addresses {
            let LookupAddress {
                socket_addr,
                protocol,
            } = addr;
            let transport = match self
                .transports()
                .select_transport(socket_addr, protocol)
                .await
            {
                Ok(selected) => selected,
                Err(_) => continue,
            };

            if transport
                .send_msg(&response.encoded, &socket_addr)
                .await
                .is_ok()
            {
                target.transport = Some(TargetTransportInfo {
                    transport,
                    socket_addr,
                });
                return Ok(());
            }
        }

        Err(Error::Transport("Failed to send response!".to_owned()))
    }

    // RFC 3261 - 18.2.2 Sending Responses
    pub(crate) fn get_response_destination(&self, request: &IncomingRequest) -> OutgoingDestInfo {
        let incoming_info = &request.incoming_info;
        let topmost_via = &incoming_info.mandatory_headers.via;
        let via_sent_by = &topmost_via.sent_by;
        let source_transport = &incoming_info.transport_msg.transport;

        if topmost_via.transport.is_reliable() {
            let source_addr = incoming_info.transport_msg.packet.source;

            let host = if let Some(ip_addr) = topmost_via.received {
                let port = via_sent_by
                    .port
                    .unwrap_or(source_transport.protocol().default_port());

                HostPort {
                    host: Host::IpAddr(ip_addr),
                    port: Some(port),
                }
            } else {
                via_sent_by.clone()
            };

            return OutgoingDestInfo {
                host_port: (host, source_transport.protocol()),
                transport: Some(TargetTransportInfo {
                    transport: source_transport.clone(),
                    socket_addr: source_addr,
                }),
            };
        }

        if let Some(maddr) = topmost_via.maddr.clone() {
            let port = via_sent_by.port.unwrap_or(5060);
            let host_port = HostPort {
                host: maddr,
                port: Some(port),
            };

            return OutgoingDestInfo {
                host_port: (host_port, topmost_via.transport),
                transport: None,
            };
        }

        if let Some(ip_addr) = topmost_via.received {
            let port = via_sent_by.port.unwrap_or(5060);
            let socket_addr = SocketAddr::new(ip_addr, port);

            return OutgoingDestInfo {
                host_port: (socket_addr.into(), source_transport.protocol()),
                transport: Some(TargetTransportInfo {
                    transport: source_transport.clone(),
                    socket_addr,
                }),
            };
        }

        OutgoingDestInfo {
            host_port: (via_sent_by.clone(), topmost_via.transport),
            transport: None,
        }
    }

    fn process_route_set<'a>(&self, request: &'a mut Request) -> Cow<'a, Uri> {
        let topmost_route = request
            .headers
            .iter_mut()
            .position(
                |header| matches!(header, Header::Route(route) if !route.name_addr.uri.lr_param),
            )
            .map(|index| {
                request
                    .headers
                    .remove(index)
                    .into_route()
                    .expect("The header must be a Route")
            });

        if topmost_route.is_some() {
            let name_addr = NameAddr::new(request.req_line.uri.clone());
            let route = Header::Route(Route {
                name_addr,
                params: Default::default(),
            });
            let index = request
                .headers
                .iter()
                .rposition(|h| matches!(h, Header::Route(_)));

            if let Some(index) = index {
                request.headers.insert(index, route);
            } else {
                request.headers.push(route);
            }
        }

        topmost_route
            .map(|route| Cow::Owned(route.name_addr.uri))
            .unwrap_or(Cow::Borrowed(&request.req_line.uri))
    }

    // RFC 3263 - 4.1 Selecting a Transport Protocol (UDP/TCP/TLS)
    // RFC 3263 - 4.2 Determining Port and IP Address (SRV/A/AAAA)
    // RFC 3261 - 12.2.1.1 Generating the Request
    // RFC 3261 - 8.1.1 Generating the Request
    // RFC 3261 - 8.1.2 Sending the Request
    pub(crate) async fn create_outgoing_request(
        &self,
        mut request: Request,
        target: Option<(TransportHandle, SocketAddr)>,
    ) -> Result<OutgoingRequest> {
        let (transport, socket_addr) = 'label: {
            if let Some(target) = target {
                target
            } else {
                let new_request_uri = self.process_route_set(&mut request);
                let addrs = self.transports().resolve_uri(&new_request_uri).await?;

                for addr in addrs {
                    let LookupAddress {
                        socket_addr,
                        protocol,
                    } = addr;

                    match self
                        .transports()
                        .select_transport(socket_addr, protocol)
                        .await
                    {
                        Ok(selected) => break 'label (selected, socket_addr),
                        Err(err) => {
                            log::error!("{}", err);
                            continue;
                        }
                    };
                }

                return Err(Error::Transport(format!(
                    "No transport found for : {}",
                    new_request_uri
                )));
            }
        };
        log::debug!(
            "Resolved target: transport={}, addr={}",
            transport.protocol().as_upper_case(),
            socket_addr
        );

        let target_info = TargetTransportInfo {
            socket_addr,
            transport,
        };

        Ok(OutgoingRequest {
            request,
            target_info,
            encoded: bytes::Bytes::new(),
        })
    }

    pub(crate) async fn on_response(&self, response: IncomingResponse) {
        log::debug!(
            "<= Response {} {} from /{}",
            response.status_line.code.as_u16(),
            response.status_line.reason.as_str(),
            response.incoming_info.transport_msg.packet.source
        );

        let mut response = Some(response);

        for plugin in self.inner.plugins.plugins() {
            plugin
                .on_incoming_response(ToTake::new(&mut response), self)
                .await;

            if response.is_none() {
                break;
            }
        }

        if let Some(response) = response {
            log::info!(
                "Response ({} {}) from /{} was unhandled by any plugin",
                response.status_line.code.as_u16(),
                response.status_line.reason.as_str(),
                response.incoming_info.transport_msg.packet.source
            );
        }
    }

    pub(crate) async fn on_request(&self, request: IncomingRequest) {
        log::debug!(
            "<= Request {} from /{}",
            request.req_line.method,
            request.incoming_info.transport_msg.packet.source
        );

        let mut request = Some(request);

        for plugin in self.inner.plugins.plugins() {
            plugin
                .on_incoming_request(ToTake::new(&mut request), self)
                .await;

            if request.is_none() {
                break;
            }
        }

        if let Some(msg) = request {
            log::debug!(
                "Request ({}, cseq={}) from /{} was unhandled",
                msg.request.req_line.method,
                msg.incoming_info.mandatory_headers.cseq.cseq(),
                msg.incoming_info.transport_msg.packet.source
            );
        }
    }

    pub(crate) fn transports(&self) -> &TransportLayer {
        &self.inner.transport
    }

    pub(crate) fn tsx_plugin(&self) -> &TsxPlugin {
        self.plugin::<TsxPlugin>()
    }

    pub(crate) fn ua_plugin(&self) -> &DialogPlugin {
        self.plugin::<DialogPlugin>()
    }

    fn from_inner(inner: Arc<EndpointInner>) -> Self {
        Self { inner }
    }
}

impl WeakEndpointHandle {
    /// Upgrade the handle to a `Endpoint`
    pub fn upgrade(&self) -> Option<Endpoint> {
        self.upgrade_to_inner().map(Endpoint::from_inner)
    }

    fn upgrade_to_inner(&self) -> Option<Arc<EndpointInner>> {
        self.0.upgrade()
    }
}
