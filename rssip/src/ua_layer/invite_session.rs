use std::ops;

use media::sdp::SessionDescription;
use media::sdp::negotiator::Negotiator;
use media::sdp::parser::SdpParser;
use tokio::sync::mpsc;

use crate::message::headers::Contact;
use crate::message::method::SipMethod;
use crate::message::status_code::StatusCode;
use crate::message::uri::{SipUri, Uri};
use crate::message::{ReasonPhrase, Request, SipBody};
use crate::transaction::{ClientTransaction, ServerTransaction};
use crate::ua_layer::dialog::Dialog;
use crate::{Endpoint, Error, IncomingRequest, IncomingResponse, Result};

// Offer                Answer             RFC    Ini Est Early
// -------------------------------------------------------------------
// 1. INVITE Req.          2xx INVITE Resp.     RFC 3261  Y   Y    N
// 2. 2xx INVITE Resp.     ACK Req.             RFC 3261  Y   Y    N

// MediaSessionConfig / MediaConfig / MediaSessionParameters

pub struct InviteSession<S> {
    state: S,
    negotiator: Negotiator,
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

impl InviteSession<Calling> {
    pub async fn send_invite(
        from_uri: SipUri,
        to_uri: SipUri,
        local_sdp: Option<SessionDescription>,
        endpoint: Endpoint,
    ) -> Result<Self> {
        let mut dialog =
            Dialog::create_uac(SipMethod::Invite, from_uri, to_uri, None, endpoint.clone())?;

        let sip_body = if let Some(sdp) = &local_sdp {
            let encoded = sdp.encode_sdp()?;
            Some(bytes::Bytes::from(encoded).into())
        } else {
            None
        };

        let negotiator = local_sdp.map_or(Negotiator::default(), |sdp| Negotiator::with_local(sdp));

        let mut request = dialog.create_request();
        request.body = sip_body;

        let client_tsx = ClientTransaction::send_request(request, endpoint.clone()).await?;

        Ok(Self {
            state: Calling {
                endpoint,
                client_tsx,
                dialog,
            },
            negotiator,
        })
    }

    pub async fn wait_answer(&mut self) -> Result<InviteSession<Established>> {
        todo!()
    }
}

impl InviteSession<Incoming> {
    pub fn from_invite(server_tsx: ServerTransaction, contact: Contact) -> Result<Self> {
        let invite = server_tsx.request();
        let endpoint = server_tsx.endpoint().clone();
        let negotiator = if let Some(body) = &invite.body {
            // EarlyOffer
            Negotiator::with_remote(Self::get_sdp(body)?)
        } else {
            // DelayedOffer
            Negotiator::default()
        };
        let dialog = Dialog::create_uas(invite, contact, endpoint.clone())?;
        Ok(InviteSession {
            state: Incoming {
                dialog,
                endpoint,
                server_tsx,
            },
            negotiator,
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
        let sdp = self.create_sdp_answer(local_sdp)?;

        let Incoming {
            server_tsx,
            mut dialog,
            endpoint,
        } = self.state;

        let mut sip_response = dialog.create_response(&server_tsx, status_code, reason_phrase);
        sip_response.body = Some(sdp);

        server_tsx.send_final_response(sip_response).await?;

        let _ack = dialog.wait_for_ack().await?;

        Ok(InviteSession {
            state: Established::new(dialog, endpoint),
            negotiator: self.negotiator,
        })
    }

    fn create_sdp_answer(&mut self, offer: SessionDescription) -> Result<SipBody> {
        self.negotiator.set_local_sdp(offer)?;
        let answer = self.negotiator.create_answer()?;
        let encoded = answer.encode_sdp()?;
        Ok(bytes::Bytes::from(encoded).into())
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
        let server_tsx = ServerTransaction::from_request(request, endpoint);

        let session = InviteSession::from_invite(server_tsx, contact);

        assert!(session.is_ok());
    }

    #[tokio::test]
    async fn test_client_session() {
        let endpoint = create_test_endpoint().await;

        let from_uri = "sip:localhost:8089".parse().unwrap();
        let to_uri = "sip:sip_homologa2.55pbx.com:5060".parse().unwrap();

        let sdp = Some(SessionDescription::default());

        let _session = InviteSession::send_invite(from_uri, to_uri, sdp, endpoint)
            .await
            .unwrap();
    }
}
