use std::sync::RwLock;

use rustc_hash::FxHashMap;
use tokio::sync::mpsc;
use tokio::time;

use crate::core::endpoint::{self, ToTake};
use crate::error::{Error, Result};
use crate::message::headers::{CSeq, CallId, Contact, From, Header, Headers, MaxForwards, To};
use crate::message::method::SipMethod;
use crate::message::params::Params;
use crate::message::status_code::StatusCode;
use crate::message::uri::{Scheme, SipUri, Uri};
use crate::message::{ReasonPhrase, Request, RequestLine};
use crate::transaction::{Role, ServerTransaction, timers};
use crate::transport::incoming::{IncomingMessage, IncomingRequest};
use crate::{
    Endpoint, IncomingResponse, OutgoingResponse, find_map_header, find_map_mut_header, headers,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogState {
    Init,
    Early,
    Confirmed,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DialogId {
    pub call_id: CallId,
    pub local_tag: String,
    pub remote_tag: Option<String>,
}

/// Represents a SIP Dialog.
pub struct Dialog {
    dialog_id: DialogId,
    state: DialogState,
    remote_cseq: Option<u32>,
    local_cseq: Option<u32>,
    from: From,
    to: To,
    contact: Contact,
    secure: bool,
    route_set: Vec<RouteSet>,
    role: Role,
    receiver: mpsc::UnboundedReceiver<IncomingMessage>,
    endpoint: Endpoint,
}

#[derive(Default)]
pub struct DialogPlugin {
    dialogs: RwLock<FxHashMap<DialogId, mpsc::UnboundedSender<IncomingMessage>>>,
}

#[async_trait::async_trait]
impl endpoint::Plugin for DialogPlugin {
    fn name(&self) -> &'static str {
        "dialog"
    }

    async fn on_incoming_request(&self, mut req: ToTake<'_, IncomingRequest>, endpoint: &Endpoint) {
        let Some(dialog_id) = DialogId::from_incoming_request(&req) else {
            return;
        };

        let request = req.take();

        let Some(channel) = self.get_dialog(&dialog_id) else {
            if request.req_line.method != SipMethod::Ack {
                let mut response = endpoint.create_outgoing_response(
                    &request,
                    StatusCode::CallOrTransactionDoesNotExist,
                    None,
                );
                if let Err(err) = endpoint.send_outgoing_response(&mut response).await {
                    log::error!("Error sending response = {err:?}");
                }
            }
            return;
        };

        // this is a mid-dialog request.
        if let Err(err) = channel.send(IncomingMessage::Request(request)) {
            log::error!("Error sending request to dialog {err}");
        }
    }

    async fn on_incoming_response(&self, _res: ToTake<'_, IncomingResponse>, _endpoint: &Endpoint) {
    }
}

impl Dialog {
    // RFC 3261 12.1.1.
    pub fn create_uas(
        request: &IncomingRequest,
        contact: Contact,
        endpoint: Endpoint,
    ) -> Result<Dialog> {
        if !request.req_line.method.can_establish_dialog() {
            return Err(Error::Dialog(format!(
                "The sip method '{}' cannot establish a dialog",
                request.req_line.method
            )));
        }
        let mandatory_headers = &request.incoming_info.mandatory_headers;
        if mandatory_headers.to.tag.is_some() {
            return Err(Error::Dialog(
                "The To header tag is added only by the server (UAS) in the response.".to_owned(),
            ));
        }
        let dialog_id = DialogId {
            call_id: mandatory_headers.call_id.clone(),
            remote_tag: mandatory_headers.from.tag().map(|t| t.to_owned()),
            local_tag: crate::generate_tag(),
        };

        let (sender, receiver) = mpsc::unbounded_channel();

        endpoint
            .ua_plugin()
            .register_dialog(dialog_id.clone(), sender);

        let dialog = Dialog {
            dialog_id,
            remote_cseq: Some(mandatory_headers.cseq.cseq()),
            local_cseq: None,
            from: mandatory_headers.from.clone(),
            to: mandatory_headers.to.clone(),
            secure: request.incoming_info.transport_msg.transport.is_secure()
                && request.req_line.uri.scheme == Scheme::Sips,
            route_set: RouteSet::from_headers(&request.headers),
            state: DialogState::Init,
            role: Role::Uas,
            contact,
            receiver,
            endpoint,
        };

        Ok(dialog)
    }

    // RFC 3261 12.1.2.
    pub fn create_uac(
        from_uri: SipUri,
        to_uri: SipUri,
        contact_uri: Option<SipUri>,
        endpoint: Endpoint,
    ) -> Dialog {
        let contact_uri = contact_uri.unwrap_or(from_uri.clone());
        let secure = to_uri.scheme() == Scheme::Sips;
        let from_tag = crate::generate_tag();
        let cseq_num = rand::random();
        let call_id = CallId::new(crate::random_str(22));
        let from = From {
            uri: from_uri,
            tag: Some(from_tag.clone()),
            params: Default::default(),
        };
        let to = To {
            uri: to_uri,
            tag: None,
            params: Default::default(),
        };
        let contact = Contact {
            uri: contact_uri,
            q: None,
            expires: None,
            param: Params::default(),
        };

        let dialog_id = DialogId {
            call_id,
            local_tag: from_tag, // MUST be set to the tag in the From field in the request
            remote_tag: None,    // MUST be set to the tag in the To field of the response
        };

        let (sender, receiver) = mpsc::unbounded_channel();

        endpoint
            .ua_plugin()
            .register_dialog(dialog_id.clone(), sender);

        let dialog = Dialog {
            dialog_id,
            remote_cseq: None,
            local_cseq: Some(cseq_num),
            from,
            to,
            secure,
            route_set: vec![],
            state: DialogState::Init,
            role: Role::Uac,
            contact,
            receiver,
            endpoint,
        };

        dialog
    }

    pub fn create_request(&mut self, method: SipMethod) -> Request {
        let local_cseq_num = self.local_cseq.get_or_insert(rand::random());
        Request {
            req_line: RequestLine {
                method,
                uri: self.contact.uri.uri().clone(),
            },
            headers: headers! {
                Header::From(self.from.clone()),
                Header::To(self.to.clone()),
                Header::CallId(self.dialog_id.call_id.clone()),
                Header::CSeq(CSeq::new(*local_cseq_num, method)),
                Header::MaxForwards(MaxForwards::new(70))
            },
            body: None,
        }
    }

    pub async fn provisional_response(
        &mut self,
        server_tsx: &mut ServerTransaction,
        status_code: StatusCode,
        reason_phrase: Option<ReasonPhrase>,
    ) -> Result<()> {
        let response = self.create_response(server_tsx, status_code, reason_phrase);

        server_tsx.send_provisional_response(response).await?;

        if status_code.as_u16() != 100 {
            self.state = DialogState::Early;
        }

        Ok(())
    }

    pub async fn final_response(
        &mut self,
        server_tsx: ServerTransaction,
        status_code: StatusCode,
    ) -> Result<()> {
        let response = self.create_response(&server_tsx, status_code, None);

        server_tsx.send_final_response(response).await?;

        Ok(())
    }

    pub(super) fn create_response(
        &self,
        server_tsx: &ServerTransaction,
        status_code: StatusCode,
        reason_phrase: Option<ReasonPhrase>,
    ) -> OutgoingResponse {
        let mut response = server_tsx.create_response(status_code, reason_phrase);
        let headers = &mut response.headers;

        let allow = self.endpoint.allow();
        let supported = self.endpoint.supported();

        let code = status_code.as_u16();

        if matches!(code, 101..=399 | 485) && !headers.iter().any(|hdr| hdr.is_contact()) {
            headers.push(Header::Contact(self.contact.clone()));
        }

        if matches!(code,180..=189 | 200..=299 | 405)
            && !allow.is_empty()
            && !headers.iter().any(|hdr| hdr.is_allow())
        {
            headers.push(Header::Allow(allow.clone()));
        }

        if matches!(code, 200..=299)
            && !supported.is_empty()
            && !headers.iter().any(|hdr| hdr.is_supported())
        {
            headers.push(Header::Supported(supported.clone()));
        }

        if code != 100 {
            let to = find_map_mut_header!(headers, To).expect("missing to header!");
            to.tag = Some(self.dialog_id.local_tag.clone());
        }

        response
    }

    pub(super) async fn receive(&mut self) -> Result<IncomingMessage> {
        let Some(msg) = self.receiver.recv().await else {
            return Err(Error::ChannelClosed);
        };

        if let IncomingMessage::Request(incoming_request) = &msg {
            self.process_incoming_request(incoming_request).await?;
        }

        Ok(msg)
    }

    pub(super) async fn receive_request(&mut self) -> Result<IncomingRequest> {
        loop {
            if let IncomingMessage::Request(req) = self.receive().await? {
                return Ok(req);
            }
            continue;
        }
    }

    pub async fn wait_for_ack(&mut self) -> Result<IncomingRequest> {
        let ack_timer = timers::T1 * 64;
        loop {
            match time::timeout(ack_timer, self.receive_request())
                .await
                .map_err(|_elapsed| Error::Custom("No ACK received".into()))??
            {
                req if req.req_line.method == SipMethod::Ack => {
                    self.state = DialogState::Confirmed;
                    return Ok(req);
                }
                req => {
                    log::debug!(
                        "received request(NoAck): {} (ignoring)",
                        req.req_line.method
                    );
                    continue;
                }
            }
        }
    }

    // RFC 3261 12.2.2.
    async fn process_incoming_request(&mut self, request: &IncomingRequest) -> Result<()> {
        let request_cseq = request.incoming_info.mandatory_headers.cseq.cseq();
        let method = request.req_line.method;

        if !matches!(method, SipMethod::Cancel | SipMethod::Ack)
            && request_cseq <= self.remote_cseq.unwrap_or(0)
        {
            let mut response = self.endpoint.create_outgoing_response(
                request,
                StatusCode::ServerInternalError,
                Some("Invalid Cseq".into()),
            );
            self.endpoint.send_outgoing_response(&mut response).await?;
            return Ok(());
        }

        self.remote_cseq = Some(request_cseq);

        // RFC3261 12.2.2.
        // When a UAS receives a target refresh request, it MUST replace the
        // dialog's remote target URI with the URI from the Contact header field
        // in that request, if present.
        if method == SipMethod::Invite
            && let Some(contact) = find_map_header!(request.headers, Contact).cloned()
        {
            self.contact = contact;
        }

        Ok(())
    }

    pub(crate) fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub(crate) fn state(&self) -> DialogState {
        self.state
    }
}

impl Drop for Dialog {
    fn drop(&mut self) {
        self.endpoint.ua_plugin().remove_dialog(&self.dialog_id);
    }
}

impl DialogPlugin {
    pub(crate) fn register_dialog(
        &self,
        dialog_id: DialogId,
        sender: mpsc::UnboundedSender<IncomingMessage>,
    ) {
        let mut dialogs = self.dialogs.write().expect("Lock failed");

        dialogs.insert(dialog_id, sender);
    }

    pub(crate) fn remove_dialog(&self, dialog_id: &DialogId) {
        let mut dialogs = self.dialogs.write().expect("Lock failed");

        dialogs.remove(dialog_id);
    }

    pub(crate) fn get_dialog(
        &self,
        dialog_id: &DialogId,
    ) -> Option<mpsc::UnboundedSender<IncomingMessage>> {
        let dialogs = self.dialogs.read().expect("Lock failed");

        dialogs.get(dialog_id).cloned()
    }
}

impl DialogId {
    pub fn from_incoming_request(request: &IncomingRequest) -> Option<Self> {
        let headers = &request.incoming_info.mandatory_headers;

        let local_tag = match headers.to.tag() {
            Some(tag) => tag.to_owned(),
            None => return None,
        };

        let call_id = headers.call_id.clone();

        let remote_tag = headers.from.tag().map(|t| t.to_owned());

        Some(Self {
            call_id,
            local_tag,
            remote_tag,
        })
    }
}

#[derive(Default)]
pub struct RouteSet {
    uri: Uri,
    params: Params,
}

impl RouteSet {
    pub fn from_headers(headers: &Headers) -> Vec<RouteSet> {
        headers
            .iter()
            .filter_map(|header| {
                if let Header::RecordRoute(route) = header {
                    Some(RouteSet {
                        uri: route.name_addr().uri.clone(),
                        params: route.params().clone(),
                    })
                } else {
                    None
                }
            })
            .collect()
    }
}
