use std::ops;

use media::sdp::SessionDescription;
use media::sdp::negotiator::SdpNegotiator;
use media::sdp::parser::SdpParser;
use tokio::sync::mpsc;

use crate::message::headers::Contact;
use crate::message::method::SipMethod;
use crate::message::status_code::StatusCode;
use crate::message::uri::SipUri;
use crate::message::{ReasonPhrase, SipBody};
use crate::sip_ua::dialog::Dialog;
use crate::transaction::{ClientTransaction, ServerTransaction};
use crate::{Endpoint, Error, IncomingRequest, IncomingResponse, Result};

// Offer                Answer             RFC    Ini Est Early
// -------------------------------------------------------------------
// 1. INVITE Req.          2xx INVITE Resp.     RFC 3261  Y   Y    N
// 2. 2xx INVITE Resp.     ACK Req.             RFC 3261  Y   Y    N

// MediaSessionConfig / MediaConfig / MediaSessionParameters

pub struct InviteSession<S> {
    state: S,
    sdp_negotiator: SdpNegotiator,
}

pub struct Incoming {
    dialog: Dialog,
    endpoint: Endpoint,
    server_tsx: ServerTransaction,
}

pub struct Calling {
    dialog: Dialog,
    endpoint: Endpoint,
    client_tsx: ClientTransaction,
}

pub struct Established {
    rx: mpsc::Receiver<InviteSessionEvent>,
}

pub enum InviteSessionEvent {
    Terminated(Cause),
    ReInvite(IncomingRequest),
    Options(IncomingRequest),
}

#[derive(Debug)]
pub enum Cause {
    ByeReceived,
}

pub struct InvitationParams {
    from_uri: SipUri,
    to_uri: SipUri,
    local_contact: Option<SipUri>,
    local_sdp: Option<SessionDescription>,
}

impl InviteSession<Calling> {
    pub async fn initiate(params: InvitationParams, endpoint: Endpoint) -> Result<Self> {
        let InvitationParams {
            from_uri,
            to_uri,
            local_contact,
            local_sdp,
        } = params;
        
        let mut dialog = Dialog::create_uac(from_uri, to_uri, local_contact, endpoint.clone());
        let mut request = dialog.create_request(SipMethod::Invite);

        if let Some(sdp) = &local_sdp {
            let encoded = sdp.encode_sdp()?;
            let sip_body = SipBody::from(bytes::Bytes::from(encoded));
            request.body = Some(sip_body);
        }
        let sdp_negotiator = local_sdp.map_or(SdpNegotiator::default(), SdpNegotiator::with_local);

        let client_tsx = ClientTransaction::send_request(request, endpoint.clone()).await?;

        let state = Calling {
            endpoint,
            client_tsx,
            dialog,
        };

        Ok(Self {
            state,
            sdp_negotiator,
        })
    }

    pub async fn receive_provisional(&mut self) -> Result<Option<IncomingResponse>> {
        let Calling { client_tsx, .. } = &mut self.state;
        let response = client_tsx.receive_provisional_response().await?;
        Ok(response)
    }

    pub async fn wait_answer(self) -> Result<InviteSession<Established>> {
        let Calling {
            mut dialog,
            endpoint,
            client_tsx,
        } = self.state;

        let response = client_tsx.receive_final_response().await?;

        match response.status_line.code.as_u16() {
            // 13.2.2.4 2xx Responses
            200..=299 => {
                let ack = dialog.create_request(SipMethod::Ack);
                let mut outgoing = endpoint.create_outgoing_request(ack, None).await?;

                endpoint.send_outgoing_request(&mut outgoing).await?;

                Ok(InviteSession {
                    state: Established::new(dialog, endpoint),
                    sdp_negotiator: self.sdp_negotiator,
                })
            }
            // 13.2.2.2 3xx Responses
            300..=399 => todo!(),
            // 13.2.2.3 4xx, 5xx and 6xx Responses
            400..=699 => todo!(),
            _ => unreachable!("The response should always have a valid final status_code"),
        }
    }
}

impl InviteSession<Incoming> {
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
        let sdp_negotiator = if let Some(body) = &request.body {
            // EarlyOffer
            SdpNegotiator::with_remote(Self::get_sdp(body)?)
        } else {
            // DelayedOffer
            SdpNegotiator::default()
        };
        let server_tsx = ServerTransaction::from_request(request, endpoint.clone());

        let state = Incoming {
            server_tsx,
            dialog,
            endpoint,
        };

        Ok(Self {
            state,
            sdp_negotiator,
        })
    }

    // RFC 3261 13.3.1.1
    pub async fn progress(
        &mut self,
        status_code: StatusCode,
        reason_phrase: Option<ReasonPhrase>,
    ) -> Result<()> {
        let Incoming {
            server_tsx, dialog, ..
        } = &mut self.state;

        dialog
            .provisional_response(server_tsx, status_code, reason_phrase)
            .await?;

        Ok(())
    }

    pub async fn accept(
        mut self,
        status_code: StatusCode,
        reason_phrase: Option<ReasonPhrase>,
        local_sdp: SessionDescription,
    ) -> Result<InviteSession<Established>> {
        let Incoming {
            server_tsx,
            mut dialog,
            endpoint,
        } = self.state;

        let mut sip_response = dialog.create_response(&server_tsx, status_code, reason_phrase);

        let offer = {
            self.sdp_negotiator.set_local_sdp(local_sdp)?;

            let answer = self.sdp_negotiator.create_answer()?;
            let sdp_str = answer.encode_sdp()?;

            SipBody::from(bytes::Bytes::from(sdp_str))
        };

        sip_response.body = Some(offer);

        server_tsx.send_final_response(sip_response).await?;

        let _ack = dialog.wait_for_ack().await?;

        Ok(InviteSession {
            state: Established::new(dialog, endpoint),
            sdp_negotiator: self.sdp_negotiator,
        })
    }
}

impl<S> InviteSession<S> {
    fn get_sdp(body: &SipBody) -> Result<SessionDescription> {
        let sdp = SdpParser::parse(body.as_ref())?;
        Ok(sdp)
    }
}

impl Established {
    fn new(dialog: Dialog, endpoint: Endpoint) -> Self {
        let (tx, rx) = mpsc::channel::<InviteSessionEvent>(10);

        tokio::spawn(async move {
            if let Err(err) = Self::session_loop(dialog, endpoint, tx).await {
                log::error!("Failed to handle dialog msg: {}", err);
            }
        });

        Self { rx }
    }

    async fn session_loop(
        mut dialog: Dialog,
        endpoint: Endpoint,
        tx: mpsc::Sender<InviteSessionEvent>,
    ) -> Result<()> {
        while let Ok(request) = dialog.receive_request().await {
            match request.req_line.method {
                SipMethod::Invite => {
                    tx.send(InviteSessionEvent::ReInvite(request))
                        .await
                        .map_err(|_| Error::ChannelClosed)?;
                    break;
                }
                SipMethod::Bye => {
                    let bye_tsx = ServerTransaction::from_request(request, endpoint);
                    dialog.final_response(bye_tsx, StatusCode::Ok).await?;

                    tx.send(InviteSessionEvent::Terminated(Cause::ByeReceived))
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

impl ops::Deref for InviteSession<Established> {
    type Target = mpsc::Receiver<InviteSessionEvent>;

    fn deref(&self) -> &Self::Target {
        &self.state.rx
    }
}

impl ops::DerefMut for InviteSession<Established> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state.rx
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::method::SipMethod;
    use crate::test_utils::{create_test_endpoint, create_test_request};
    use crate::transport::{MockTransport, TransportHandle};

    fn create_test_invite() -> IncomingRequest {
        let transport = TransportHandle::new(MockTransport::new_udp());
        create_test_request(SipMethod::Invite, transport)
    }

    #[tokio::test]
    async fn test_session_with_late_offer() {
        let endpoint = create_test_endpoint().await;
        let request = create_test_invite();
        let contact = "test <sip:localhost:5969>".parse().unwrap();

        let session = InviteSession::from_invite(request, contact, endpoint);

        assert!(session.is_ok());
    }

    #[tokio::test]
    async fn test_client_session() {
        let endpoint = create_test_endpoint().await;

        let from_uri = "sip:localhost:8089".parse().unwrap();
        let to_uri = "sip:sip_homologa2.55pbx.com:5060".parse().unwrap();

        let local_sdp = Some(SessionDescription::default());

        let inv_params = InvitationParams {
            from_uri,
            to_uri,
            local_contact: None,
            local_sdp,
        };

        let mut session = InviteSession::initiate(inv_params, endpoint).await.unwrap();

        while let Some(provisional) = session.receive_provisional().await.unwrap() {
            let code = provisional.status_line.code.as_u16();
            println!("received provisional: {}", code);
        }
        let _session = session.wait_answer().await.unwrap();
    }
}
