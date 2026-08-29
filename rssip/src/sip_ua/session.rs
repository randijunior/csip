use media::media::{MediaParams, MediaSession};
use media::negotiator::{Negotiator, NegotiatorState};
use media::sdp::SessionDescription;
use media::sdp::parser::SdpParser;
use tokio::sync::mpsc;
use utils::encode::Encode;

use crate::message::headers::{Contact, ContentType, Header};
use crate::message::method::SipMethod;
use crate::message::status_code::StatusCode;
use crate::message::uri::SipUri;
use crate::message::{ReasonPhrase, SipBody};
use crate::sip_ua::dialog::Dialog;
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
    media_params: Option<MediaParams>,
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
    Media(MediaEvent)
}


pub enum MediaEvent {
    RtpReceived(media::rtp::packet::RtpPacket)
}


#[derive(Debug, Clone, Copy)]
pub enum Cause {
    ByeReceived,
}

pub struct InvitationParams {
    pub from_uri: SipUri,
    pub to_uri: SipUri,
    pub contact: Option<SipUri>,
}

impl<S> Session<S> {
    fn parse_sdp(body: &SipBody) -> Result<SessionDescription> {
        let sdp = SdpParser::parse(body.as_ref())?;
        Ok(sdp)
    }
    pub fn set_media_params(&mut self, params: MediaParams) {
        self.media_params = Some(params);
    }
}

impl Session<Calling> {
    // RFC 3261 13.2.1
    pub async fn send_invite(
        inv_params: InvitationParams,
        media_params: Option<MediaParams>,
        endpoint: Endpoint,
    ) -> Result<Self> {
        let InvitationParams {
            from_uri,
            to_uri,
            contact,
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

        if let Some(params) = &media_params {
            let offer = negotiator.create_offer(params)?;
            let sdp_str = offer.encode()?;

            negotiator.set_local_offer(offer)?;

            let sip_body = SipBody::from(bytes::Bytes::from(sdp_str));

            request
                .headers
                .push(Header::ContentType(ContentType::new_sdp()));

            request.body = Some(sip_body);
        }

        let client_tsx = ClientTransaction::send_request(request, endpoint).await?;

        Ok(Self {
            state: Calling { client_tsx, dialog },
            negotiator,
            media_params,
        })
    }

    pub async fn receive_provisional(&mut self) -> Result<Option<IncomingResponse>> {
        let Calling { client_tsx, .. } = &mut self.state;
        let response = client_tsx.receive_provisional_response().await?;
        Ok(response)
    }

    pub async fn receive_answer(mut self) -> Result<Session<Established>> {
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
                            let Some(media_params) = &self.media_params else {
                                return Err(Error::Custom(
                                    "media_params required to answer a delayed offer".into(),
                                ));
                            };
                            let local = negotiator.create_offer(media_params)?;

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
                let params = self.media_params.as_ref().unwrap();

                Ok(Session {
                    state: Established::create(dialog, params).await?,
                    negotiator: self.negotiator,
                    media_params: self.media_params,
                })
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
            media_params: None,
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
    ) -> Result<Session<Established>> {
        let Incoming {
            server_tsx,
            mut dialog,
        } = self.state;

        let Some(media_params) = &self.media_params else {
                return Err(Error::Custom(
                    "media_params required to accept a early offer".into(),
                ));
        };

        let mut sip_response = dialog.create_response(&server_tsx, status_code, reason_phrase);

        let body = if self.negotiator.remote_offer().is_some() {

            let local = self.negotiator.create_offer(media_params)?;

            self.negotiator.set_local_offer(local)?;

            let answer = self.negotiator.create_answer()?;

            SipBody::from(bytes::Bytes::from(answer.encode()?))
        } else {
            todo!()
        };

        sip_response.body = Some(body);

        server_tsx.send_final_response(sip_response).await?;

        let _ack = dialog.wait_for_ack().await?;


         Ok(Session {
             state: Established::create(dialog, media_params).await?,
             negotiator: self.negotiator,
             media_params: self.media_params,
         })
    }
}

impl Established {
    async fn create(dialog: Dialog, params: &MediaParams) -> Result<Self> {
        let media = MediaSession::from_media_params(params).await?;

        let (tx, rx) = mpsc::channel::<SessionEvent>(10);
        tokio::spawn(async move {
            if let Err(err) = Self::session_loop(dialog, media,tx).await {
                log::error!("Failed to handle dialog msg: {}", err);
            }
        });

        Ok(Self { rx })
    }

    async fn session_loop(mut dialog: Dialog, media: MediaSession, tx: mpsc::Sender<SessionEvent>) -> Result<()> {
        while let Ok(request) = dialog.receive_request().await {
            match request.req_line.method {
                SipMethod::Invite => {
                    tx.send(SessionEvent::ReInvite(request))
                        .await
                        .map_err(|_| Error::ChannelClosed)?;
                    continue;
                }
                SipMethod::Bye => {
                    let endpoint = dialog.endpoint().clone();
                    let bye_tsx = ServerTransaction::from_request(request, endpoint);

                    dialog.final_response(bye_tsx, StatusCode::Ok).await?;

                    tx.send(SessionEvent::Terminated(Cause::ByeReceived))
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
}

impl Session<Established> {
    pub async fn next_event(&mut self) -> Option<SessionEvent> {
        self.state.rx.recv().await
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use crate::message::method::SipMethod;
    use crate::test_utils::{create_test_endpoint, create_test_request};
    use crate::transport::{MockTransport, TransportHandle};

    fn create_test_invite() -> IncomingRequest {
        let transport = TransportHandle::new(MockTransport::new_udp());
        create_test_request(SipMethod::Invite, transport)
    }

    #[tokio::test]
    async fn test_server_session_without_sdp() {
        let endpoint = create_test_endpoint().await;
        let request = create_test_invite();

        let contact = "test <sip:localhost:8089>".parse().unwrap();

        let _session = Session::from_invite(request, contact, endpoint)
            .expect("Failed to create session from invite");
    }

    #[tokio::test]
    async fn test_client_session_send_invite_without_sdp() {
        let endpoint = create_test_endpoint().await;

        let from_uri = SipUri::from_str("Alice <sip:alice@example.com>").unwrap();
        let to_uri = SipUri::from_str("Bob <sip:bob@example.com>").unwrap();

        let params = InvitationParams {
            from_uri: from_uri.clone(),
            to_uri: to_uri.clone(),
            contact: None,
        };

        let _session = Session::send_invite(params, None, endpoint).await.unwrap();
    }
}
