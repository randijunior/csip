use std::ops;

use media::sdp::SessionDescription;
use media::sdp::negotiator::{Negotiator, SdpOfferParams};
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
}

#[derive(Debug)]
pub enum Cause {
    ByeReceived,
}

pub struct InvitationParams {
    pub from_uri: SipUri,
    pub to_uri: SipUri,
    pub contact: Option<SipUri>,
}

impl Session<Calling> {
    // RFC 3261 13.2.1
    pub async fn send_invite(
        params: InvitationParams,
        sdp_params: Option<SdpOfferParams>,
        endpoint: Endpoint,
    ) -> Result<Self> {
        let InvitationParams {
            from_uri,
            to_uri,
            contact,
        } = params;

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

        let negotiator = if let Some(sdp) = sdp_params {
            let mut nego = Negotiator::default();

            let sdp = nego.generate_offer(sdp).encode()?;
            let sip_body = SipBody::from(bytes::Bytes::from(sdp));

            request
                .headers
                .push(Header::ContentType(ContentType::new_sdp()));
            request.body = Some(sip_body);

            nego
        } else {
            Negotiator::default()
        };

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

    pub async fn wait_answer(self) -> Result<Session<Established>> {
        let Calling {
            mut dialog,
            client_tsx,
        } = self.state;

        let response = client_tsx.receive_final_response().await?;

        match response.status_line.code.as_u16() {
            // 13.2.2.4 2xx Responses
            200..=299 => {
                let ack = dialog.create_request(SipMethod::Ack);
                let endpoint = dialog.endpoint();

                let mut outgoing = endpoint.create_outgoing_request(ack, None).await?;

                endpoint.send_outgoing_request(&mut outgoing).await?;

                Ok(Session {
                    state: Established::new(dialog),
                    negotiator: self.negotiator,
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
        let negotiator = if let Some(body) = &request.body {
            // EarlyOffer
            Negotiator::with_remote(Self::get_sdp(body)?)
        } else {
            // DelayedOffer
            Negotiator::default()
        };
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

        let offer = {
            let _offer = self.negotiator.generate_offer(sdp_params);

            let answer = self.negotiator.generate_answer()?;
            let sdp = answer.encode()?;

            SipBody::from(bytes::Bytes::from(sdp))
        };

        sip_response.body = Some(offer);

        server_tsx.send_final_response(sip_response).await?;

        let _ack = dialog.wait_for_ack().await?;

        Ok(Session {
            state: Established::new(dialog),
            negotiator: self.negotiator,
        })
    }
}

impl<S> Session<S> {
    fn get_sdp(body: &SipBody) -> Result<SessionDescription> {
        let sdp = SdpParser::parse(body.as_ref())?;
        Ok(sdp)
    }
}

impl Established {
    fn new(dialog: Dialog) -> Self {
        let (tx, rx) = mpsc::channel::<SessionEvent>(10);

        tokio::spawn(async move {
            if let Err(err) = Self::session_loop(dialog, tx).await {
                log::error!("Failed to handle dialog msg: {}", err);
            }
        });

        Self { rx }
    }

    async fn session_loop(mut dialog: Dialog, tx: mpsc::Sender<SessionEvent>) -> Result<()> {
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
