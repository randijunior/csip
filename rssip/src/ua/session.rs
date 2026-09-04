use std::net::SocketAddr;

use media::negotiator::{Negotiator, NegotiatorState, SdpOfferParams};
use media::rtp::session::RtpSession;
use media::sdp::SessionDescription;
use media::sdp::parser::SdpParser;
use tokio::sync::mpsc;
use utils::encode::Encode;

use crate::message::headers::{Contact, ContentType, Header};
use crate::message::method::SipMethod;
use crate::message::status_code::StatusCode;
use crate::message::uri::SipUri;
use crate::message::{ReasonPhrase, SipBody};
use crate::ua::dialog::{Dialog, DialogState};
use crate::transaction::{ClientTransaction, ServerTransaction};
use crate::{Endpoint, Error, IncomingRequest, IncomingResponse, OutgoingRequest, Result};

// Offer                Answer             RFC    Ini Est Early
// -------------------------------------------------------------------
// 1. INVITE Req.          2xx INVITE Resp.     RFC 3261  Y   Y    N
// 2. 2xx INVITE Resp.     ACK Req.             RFC 3261  Y   Y    N

/// Represents a SIP Session.
pub struct Session<S> {
    state: S,
    negotiator: Negotiator,
}

pub struct Incoming {
    dialog: Dialog,
    server_tsx: ServerTransaction,
}

pub struct Calling {
    dialog: Dialog,
    client_tsx: ClientTransaction,
}

pub struct Established {
    rx: mpsc::Receiver<SessionEvent>,
}

pub enum SessionEvent {
    Terminated(Cause),
    ReInvite(IncomingRequest),
    Options(IncomingRequest),
    Media(MediaEvent),
}

pub enum MediaEvent {
    RtpSession(RtpSession),
}

#[derive(Debug, Clone, Copy)]
pub enum Cause {
    ByeReceived,
}

pub struct InvitationParams {
    pub from_uri: SipUri,
    pub to_uri: SipUri,
    pub contact: Option<SipUri>,
    pub sdp: Option<SdpOfferParams>,
}

impl<S> Session<S> {
    fn parse_sdp(body: &SipBody) -> Result<SessionDescription> {
        let sdp = SdpParser::parse(body.as_ref())?;
        Ok(sdp)
    }
}

impl Session<Calling> {
    // RFC 3261 13.2.1
    pub async fn send_invite(inv_params: InvitationParams, endpoint: Endpoint) -> Result<Self> {
        let InvitationParams {
            from_uri,
            to_uri,
            contact,
            sdp,
        } = inv_params;

        let mut dialog = Dialog::create_uac(from_uri, to_uri, contact, endpoint.clone());

        let mut request = dialog.create_request(SipMethod::Invite);

        let allow = endpoint.allow();
        let supported = endpoint.supported();

        if !allow.is_empty() {
            request.headers.push(Header::Allow(allow.clone()));
        }

        if !supported.is_empty() {
            request.headers.push(Header::Supported(supported.clone()));
        }

        let mut negotiator = Negotiator::default();

        if let Some(params) = sdp {
            let offer = negotiator.create_offer(params)?;
            let encoded = offer.encode()?;

            negotiator.set_local_offer(offer)?;

            let sip_body = SipBody::from(bytes::Bytes::from(encoded));

            request
                .headers
                .push(Header::ContentType(ContentType::new_sdp()));

            request.body = Some(sip_body);
        }

        let client_tsx = ClientTransaction::send_request(request, endpoint).await?;

        Ok(Self {
            state: Calling { client_tsx, dialog },
            negotiator,
        })
    }

    pub async fn receive_provisional(&mut self) -> Result<Option<IncomingResponse>> {
        let Calling { client_tsx, .. } = &mut self.state;
        let response = client_tsx.receive_provisional_response().await?;
        Ok(response)
    }

    pub async fn receive_answer(
        mut self,
        offer: Option<SdpOfferParams>,
    ) -> Result<Session<Established>> {
        let Calling {
            mut dialog,
            client_tsx,
        } = self.state;

        let response = client_tsx.receive_final_response().await?;

        match response.status_line.code.as_u16() {
            // 13.2.2.4 2xx Responses
            200..=299 => {
                let ack_body = if let Some(body) = &response.body {
                    let negotiator = &mut self.negotiator;
                    let remote = Self::parse_sdp(body)?;

                    match negotiator.state() {
                        NegotiatorState::Initial => {
                            let Some(params) = offer else {
                                return Err(Error::Custom(
                                    "offer required to answer a delayed offer".into(),
                                ));
                            };
                            let local = negotiator.create_offer(params)?;

                            negotiator.set_local_offer(local)?;
                            negotiator.set_remote_offer(remote)?;

                            let answer = negotiator.create_answer()?;

                            Some(SipBody::from(bytes::Bytes::from(answer.encode()?)))
                        }
                        NegotiatorState::LocalOffer => {
                            // This is an answer.
                            negotiator.process_answer(remote)?;

                            None
                        }
                        NegotiatorState::RemoteOffer => todo!("we have early offer?"),
                        _ => unreachable!(),
                    }
                } else {
                    None
                };

                let mut ack = dialog.create_request(SipMethod::Ack);
                ack.body = ack_body;

                let endpoint = dialog.endpoint();

                let mut outgoing = endpoint.create_outgoing_request(ack, None).await?;

                endpoint.send_outgoing_request(&mut outgoing).await?;

                // Create Media Session Here? Or let to the application user
                // ned to create UDP server

                Ok(Session::create(self.negotiator, dialog))
            }
            // 13.2.2.2 3xx Responses
            300..=399 => todo!(),
            // 13.2.2.3 4xx, 5xx and 6xx Responses
            400..=699 => todo!(),
            _ => unreachable!("The response should always have a valid final status_code"),
        }
    }

    pub fn request(&self) -> &OutgoingRequest {
        self.state.client_tsx.request()
    }
}

impl Session<Incoming> {
    pub fn from_invite(
        request: IncomingRequest,
        contact: Contact,
        endpoint: Endpoint,
    ) -> Result<Self> {
        if request.req_line.method != SipMethod::Invite {
            return Err(Error::Custom(format!(
                "unexpected method '{}' expected INVITE",
                request.req_line.method
            )));
        }
        let dialog = Dialog::create_uas(&request, contact, endpoint.clone())?;

        let mut negotiator = Negotiator::new();

        if let Some(body) = &request.body {
            // EarlyOffer
            let remote_offer = Self::parse_sdp(body)?;

            negotiator.set_remote_offer(remote_offer)?;
        }

        let server_tsx = ServerTransaction::from_request(request, endpoint);

        Ok(Self {
            state: Incoming { server_tsx, dialog },
            negotiator,
        })
    }

    // RFC 3261 13.3.1.1
    pub async fn progress(
        &mut self,
        status_code: StatusCode,
        reason_phrase: Option<ReasonPhrase>,
    ) -> Result<()> {
        let Incoming { server_tsx, dialog } = &mut self.state;

        dialog
            .provisional_response(server_tsx, status_code, reason_phrase)
            .await?;

        Ok(())
    }

    pub async fn accept(
        mut self,
        status_code: StatusCode,
        reason_phrase: Option<ReasonPhrase>,
        sdp_params: SdpOfferParams,
    ) -> Result<Session<Established>> {
        let Incoming {
            server_tsx,
            mut dialog,
        } = self.state;

        let mut sip_response = dialog.create_response(&server_tsx, status_code, reason_phrase);

        // If the INVITE request contained an offer, and the UAS had not yet
        // sent an answer, the 2xx MUST contain an answer.  If the INVITE did
        // not contain an offer, the 2xx MUST contain an offer if the UAS had
        // not yet sent an offer.
        let offer = self.negotiator.create_offer(sdp_params)?;

        let body = if self.negotiator.state() == NegotiatorState::RemoteOffer {
            self.negotiator.set_local_offer(offer)?;
            let answer = self.negotiator.create_answer()?;
            answer.encode()?
        } else {
            let encoded = offer.encode()?;
            // Must be NegotiatorState::Initial
            self.negotiator.set_local_offer(offer)?;
            encoded
        };

        sip_response.body = Some(SipBody::from(bytes::Bytes::from(body)));

        server_tsx.send_final_response(sip_response).await?;

        let ack = dialog.wait_for_ack().await?;

        if self.negotiator.state() == NegotiatorState::LocalOffer {
            let Some(body) = &ack.body else {
                return Err(Error::Custom("missing answer on ack".into()));
            };
            let answer = Self::parse_sdp(body)?;
            self.negotiator.process_answer(answer)?;
        }

        Ok(Session::create(self.negotiator, dialog))
    }
}

impl Session<Established> {
    fn create(negotiator: Negotiator, dialog: Dialog) -> Self {
        let (tx, rx) = mpsc::channel::<SessionEvent>(10);

        let sdp = negotiator.local_offer().expect("a offer");

        // accepted stream(s)
        let rtp_sockets: Vec<SocketAddr> = sdp
            .media
            .iter()
            .map(|media| {
                let ip = media
                    .connection_information
                    .as_ref()
                    .map(|c| c.conection_address)
                    .unwrap_or(sdp.origin.unicast_address);
                SocketAddr::new(ip, media.port)
                // need codec
            })
            .collect();

        tokio::spawn(async move {
            if let Err(err) = Self::session_loop(dialog, rtp_sockets, tx).await {
                log::error!("Failed to handle dialog msg: {}", err);
            }
        });

        Self {
            state: Established { rx },
            negotiator,
        }
    }

    async fn session_loop(
        mut dialog: Dialog,
        rtp_sockets: Vec<SocketAddr>,
        session_tx: mpsc::Sender<SessionEvent>,
    ) -> Result<()> {
        for rtp_sock in rtp_sockets {
            let rtp_session = RtpSession::create(rtp_sock).await?;
            session_tx
                .send(SessionEvent::Media(MediaEvent::RtpSession(rtp_session)))
                .await
                .map_err(|_| Error::ChannelClosed)?;
        }

        while let Ok(request) = dialog.receive_request().await {
            match request.req_line.method {
                SipMethod::Invite => {
                    session_tx
                        .send(SessionEvent::ReInvite(request))
                        .await
                        .map_err(|_| Error::ChannelClosed)?;
                    continue;
                }
                SipMethod::Bye => {
                    let endpoint = dialog.endpoint().clone();
                    let bye_tsx = ServerTransaction::from_request(request, endpoint);

                    dialog.final_response(bye_tsx, StatusCode::Ok).await?;

                    session_tx
                        .send(SessionEvent::Terminated(Cause::ByeReceived))
                        .await
                        .map_err(|_| Error::ChannelClosed)?;

                    break;
                }
                method => {
                    log::debug!("received request: {} (ignoring)", method);
                    continue;
                }
            }
        }

        Ok(())
    }

    pub async fn next_event(&mut self) -> Option<SessionEvent> {
        self.state.rx.recv().await
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use media::codec::Codec;
    use media::negotiator::SdpMediaStream;
    use media::sdp::{Direction, SdpTransport};

    use super::*;
    use crate::message::method::SipMethod;
    use crate::test_utils::{create_test_endpoint, create_test_request};
    use crate::transport::{MockTransport, TransportHandle};

    fn create_test_invite() -> IncomingRequest {
        let transport = TransportHandle::new(MockTransport::new_udp());
        create_test_request(SipMethod::Invite, transport)
    }

    fn create_test_inv_params() -> InvitationParams {
        InvitationParams {
            from_uri: "Alice <sip:alice@example.com>".parse().unwrap(),
            to_uri: "Bob <sip:bob@example.com>".parse().unwrap(),
            contact: None,
            sdp: None,
        }
    }

    #[tokio::test]
    async fn test_server_session_late_offer() {
        let endpoint = create_test_endpoint().await;
        let request = create_test_invite();
        let contact = "test <sip:localhost:8089>".parse().unwrap();

        let _session = Session::from_invite(request, contact, endpoint).unwrap();
    }

    #[tokio::test]
    async fn test_client_session_late_offer() {
        let endpoint = create_test_endpoint().await;
        let params = create_test_inv_params();

        let _session = Session::send_invite(params, endpoint).await.unwrap();
    }

    #[tokio::test]
    async fn test_server_session_accept_invite_with_offer() {
        let endpoint = create_test_endpoint().await;
        let request = create_test_invite();

        let contact = "test <sip:localhost:8089>".parse().unwrap();

        let mut session = Session::from_invite(request, contact, endpoint).unwrap();

        session.progress(StatusCode::Trying, None).await.unwrap();

        let sdp = SdpOfferParams::new(IpAddr::from([127, 0, 0, 1]), Direction::SendRecv);

        let sdp = sdp.add_media_stream(
            SdpMediaStream::audio(34391, SdpTransport::RTPAVP)
                .with_codecs(vec![Codec::ULAW, Codec::ALAW]),
        );

        let _session = session.accept(StatusCode::Ok,None, sdp).await.unwrap();
    }
}
