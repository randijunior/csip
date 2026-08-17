use std::assert_matches;
use std::net::SocketAddr;

use tokio::sync::mpsc::{self};
use tokio::time::{self, Instant};
use utils::PeekableReceiver;

use crate::error::{Error, TransactionError};
use crate::message::Request;
use crate::message::headers::via::Rport;
use crate::message::headers::{CSeq, Header, Via};
use crate::message::method::SipMethod;
use crate::transaction::fsm::{State, StateMachine};
use crate::transaction::timers::{T1, T4};
use crate::transaction::{Role, TransactionKey};
use crate::transport::TransportHandle;
use crate::transport::incoming::{IncomingMessage, IncomingResponse, MandatoryHeaders};
use crate::transport::outgoing::OutgoingRequest;
use crate::{Endpoint, Result, find_map_mut_header};

/// A Client Transaction.
///
/// Represents a SIP client transaction.
pub struct ClientTransaction {
    key: TransactionKey,
    endpoint: Endpoint,
    state_machine: StateMachine,
    request: OutgoingRequest,
    channel: PeekableReceiver<IncomingMessage>,
    timeout: Instant,
}

impl ClientTransaction {
    pub async fn send_request(request: Request, endpoint: Endpoint) -> Result<Self> {
        Self::send(request, endpoint, None).await
    }

    pub(crate) async fn send_request_with_target(
        request: Request,
        endpoint: Endpoint,
        target: (TransportHandle, SocketAddr),
    ) -> Result<Self> {
        Self::send(request, endpoint, Some(target)).await
    }

    async fn send(
        request: Request,
        endpoint: Endpoint,
        target: Option<(TransportHandle, SocketAddr)>,
    ) -> Result<Self> {
        let method = request.req_line.method;
        assert_ne!(
            method,
            SipMethod::Ack,
            "ACK requests do not create transactions"
        );
        let mut outgoing = endpoint.create_outgoing_request(request, target).await?;
        let headers = &mut outgoing.request.headers;

        let via = match find_map_mut_header!(headers, Via) {
            Some(via) => via,
            None => {
                let sent_by = outgoing.target_info.transport.local_addr().into();
                let transport = outgoing.target_info.transport.protocol();
                let branch = crate::generate_branch();
                let via =
                    Via::new_with_transport(transport, sent_by, Some(branch), Some(Rport(None)));

                let header = headers.insert_mut(0, Header::Via(via));

                header
                    .as_via_mut()
                    .expect("The 'insert_mut' will return the same 'Via' header")
            }
        };
        let branch = match via.branch.as_ref().map(|b| b.to_owned()) {
            Some(branch) => branch,
            None => {
                let branch = crate::generate_branch();
                via.branch = Some(branch.clone());
                branch
            }
        };
        let key = TransactionKey::new_key_3261(Role::Uac, method, branch);

        endpoint.send_outgoing_request(&mut outgoing).await?;

        let state = if method == SipMethod::Invite {
            State::Calling
        } else {
            State::Trying
        };
        let (sender, channel) = mpsc::channel(10);

        endpoint.tsx_plugin().add_transaction(key.clone(), sender);

        let client_tsx = Self {
            key,
            endpoint,
            state_machine: StateMachine::new(state),
            channel: channel.into(),
            request: outgoing,
            timeout: Instant::now() + T1 * 64,
        };

        log::trace!(
            "Client Transaction Created [{:#?}] ({:p})",
            method,
            &client_tsx
        );

        Ok(client_tsx)
    }

    pub(crate) fn create_ack_request(
        &self,
        outgoing: &OutgoingRequest,
        response: &IncomingResponse,
    ) -> OutgoingRequest {
        assert_matches!(
            response.status_line.code.as_u16(),
            300..699,
            "Invalid status code for ACK, must be a 300-699 final response"
        );
        let target = outgoing.request.req_line.uri.clone();
        // Clone: Via, To, From, Max-Forwards, Call-ID and CSeq from response.
        let headers = MandatoryHeaders {
            cseq: CSeq::new(
                response.incoming_info.mandatory_headers.cseq.cseq(),
                SipMethod::Ack,
            ),
            ..response.incoming_info.mandatory_headers.clone()
        }
        .into_headers();

        let request = Request::with_headers(SipMethod::Ack, target, headers);
        let target_info = outgoing.target_info.clone();

        OutgoingRequest {
            request,
            target_info,
            encoded: Default::default(),
        }
    }

    pub async fn receive_provisional_response(&mut self) -> Result<Option<IncomingResponse>> {
        let state = self.state();

        if state <= State::Proceeding {
            let response = self.receive_provisional().await?;

            if state != State::Proceeding {
                self.state_machine.set_state(State::Proceeding);
            }

            Ok(response)
        } else {
            Ok(None)
        }
    }

    pub async fn receive_final_response(mut self) -> Result<IncomingResponse> {
        let response = self.receive_final().await?;

        let is_invite_tsx = self.request.req_line.method == SipMethod::Invite;

        if self.is_reliable()
            || (is_invite_tsx
                && matches!(response.status_line.code.as_u16(), 200..299)
                && matches!(self.state(), State::Calling | State::Proceeding))
        {
            self.state_machine.set_state(State::Terminated);
            return Ok(response);
        }

        self.state_machine.set_state(State::Completed);

        let (timer, mut ack_request) = if is_invite_tsx {
            let mut ack_request = self.create_ack_request(&self.request, &response);
            self.endpoint
                .send_outgoing_request(&mut ack_request)
                .await?;

            (Instant::now() + 64 * T1, Some(ack_request))
        } else {
            (Instant::now() + T4, None)
        };

        tokio::spawn(async move {
            while let Ok(Some(IncomingMessage::Response(_))) =
                time::timeout_at(timer, self.channel.recv()).await
            {
                if let Some(ref mut ack) = ack_request
                    && let Err(err) = self.endpoint.send_outgoing_request(ack).await
                {
                    log::error!("Failed to retransmit ack: {}", err);
                }
            }
            self.state_machine.set_state(State::Terminated);
        });

        Ok(response)
    }

    async fn receive_provisional(&mut self) -> Result<Option<IncomingResponse>> {
        self.receive_response_if(|res| res.status_line.code.is_provisional())
            .await
    }

    async fn receive_final(&mut self) -> Result<IncomingResponse> {
        self.receive_response_if(|res| res.status_line.code.is_final())
            .await?
            .ok_or(Error::ChannelClosed)
    }

    async fn receive_response_if(
        &mut self,
        pred: fn(&IncomingResponse) -> bool,
    ) -> Result<Option<IncomingResponse>> {
        let deadline = self.timeout;

        match self.state() {
            State::Calling | State::Trying if self.is_unreliable() => {
                let mut retrans_interval = T1;
                loop {
                    let fut = time::timeout(retrans_interval, self.recv_if(pred));

                    match time::timeout_at(deadline, fut).await {
                        Ok(Ok(Some(msg))) => return Ok(Some(msg)),
                        Ok(Err(_elapsed)) => {
                            // retransmit
                            self.endpoint
                                .send_outgoing_request(&mut self.request)
                                .await?;
                            retrans_interval *= 2;
                            continue;
                        }
                        Err(_elapsed) => {
                            self.state_machine.set_state(State::Terminated);
                            return Err(TransactionError::Timeout.into());
                        }
                        Ok(Ok(None)) => return Ok(None),
                    }
                }
            }
            State::Calling | State::Trying => {
                match time::timeout_at(deadline, self.recv_if(pred)).await {
                    Ok(Some(msg)) => Ok(Some(msg)),
                    Err(_elapsed) => {
                        self.state_machine.set_state(State::Terminated);
                        Err(TransactionError::Timeout.into())
                    }
                    Ok(None) => Ok(None),
                }
            }
            State::Proceeding if self.request.req_line.method == SipMethod::Invite => {
                match self.recv_if(pred).await {
                    Some(msg) => Ok(Some(msg)),
                    _ => Ok(None),
                }
            }
            State::Proceeding => match time::timeout_at(deadline, self.recv_if(pred)).await {
                Ok(Some(msg)) => Ok(Some(msg)),
                Err(_elapsed) => {
                    self.state_machine.set_state(State::Terminated);
                    Err(TransactionError::Timeout.into())
                }
                Ok(None) => Ok(None),
            },
            _ => unreachable!(),
        }
    }

    async fn recv_if(&mut self, cond: fn(&IncomingResponse) -> bool) -> Option<IncomingResponse> {
        let cond = |msg: &IncomingMessage| matches!(msg, IncomingMessage::Response(response) if cond(response));

        match self.channel.recv_if(cond).await {
            Some(IncomingMessage::Response(res)) => Some(res),
            _ => None,
        }
    }

    pub(crate) fn state(&self) -> State {
        self.state_machine.state()
    }

    pub fn state_machine_mut(&mut self) -> &mut StateMachine {
        &mut self.state_machine
    }

    pub fn key(&self) -> &TransactionKey {
        &self.key
    }

    fn is_reliable(&self) -> bool {
        self.request.target_info.transport.is_reliable()
    }

    fn is_unreliable(&self) -> bool {
        self.request.target_info.transport.is_unreliable()
    }
    pub(crate) fn request(&self) -> &OutgoingRequest {
        &self.request
    }
}

impl Drop for ClientTransaction {
    fn drop(&mut self) {
        self.endpoint.tsx_plugin().remove_transaction(&self.key);
        log::trace!("Transaction Destroyed [{:#?}] ({:p})", Role::Uac, &self);
    }
}

/// Unit tests
#[cfg(test)]
mod tests {
    use super::*;
    use crate::macros::assert_eq_tsx_state as assert_eq_state;
    use crate::message::status_code::StatusCode;
    use crate::test_utils;
    use crate::transaction::fsm;
    use crate::transaction::timers::MockRetransTimer;
    use crate::transport::incoming::{IncomingInfo, IncomingRequest};
    use crate::transport::{MockTransport, Packet, TransportMessage};

    struct ClientTestContext {
        client: ClientTransaction,
        server: MockServerTsx,
        transport: MockTransport,
        state: fsm::TsxStateChangeReceiver,
        timer: MockRetransTimer,
    }

    struct MockServerTsx {
        sender: mpsc::Sender<IncomingMessage>,
        request: IncomingRequest,
        endpoint: Endpoint,
    }

    impl ClientTestContext {
        async fn setup(method: SipMethod) -> Self {
            Self::new(method, MockTransport::new_udp()).await
        }

        async fn setup_reliable(method: SipMethod) -> Self {
            Self::new(method, MockTransport::new_tcp()).await
        }

        async fn new(method: SipMethod, transport: MockTransport) -> Self {
            let tp_handle = TransportHandle::new(transport.clone());
            let timer = MockRetransTimer::new();

            let endpoint = test_utils::create_test_endpoint().await;
            let incoming = test_utils::create_test_request(method, tp_handle.clone());

            let destination = incoming.incoming_info.transport_msg.packet.source;

            let target = (tp_handle, destination);

            let mut client = ClientTransaction::send_request_with_target(
                incoming.request.clone(),
                endpoint.clone(),
                target,
            )
            .await
            .expect("failure sending request");

            let expected_state = if method == SipMethod::Invite {
                fsm::State::Calling
            } else {
                fsm::State::Trying
            };

            assert_eq!(
                client.state(),
                expected_state,
                "Transaction state should transition to {expected_state} after sending request"
            );

            let sender = endpoint.tsx_plugin().get_entry(client.key()).unwrap();

            let server = MockServerTsx {
                sender,
                request: incoming,
                endpoint,
            };

            let state = client.state_machine_mut().subscribe_state();

            Self {
                client,
                server,
                transport,
                timer,
                state,
            }
        }
    }

    impl MockServerTsx {
        pub async fn respond(&self, code: StatusCode) {
            let mandatory_headers = self.request.incoming_info.mandatory_headers.clone();
            let outgoing = self
                .endpoint
                .create_outgoing_response(&self.request, code, None);

            let packet = Packet::new(
                outgoing.encoded,
                self.request.incoming_info.transport_msg.packet.source,
            );

            let transport_msg = TransportMessage {
                packet,
                transport: self.request.incoming_info.transport_msg.transport.clone(),
            };
            let info = IncomingInfo {
                transport_msg,
                mandatory_headers,
            };

            let response = IncomingResponse {
                response: outgoing.response,
                incoming_info: Box::new(info),
            };

            let transaction_message = IncomingMessage::Response(response);

            self.sender.send(transaction_message).await.unwrap();
        }
    }

    #[tokio::test(start_paused = true)]
    async fn invite_should_not_start_timer_a_when_transport_is_reliable() {
        let mut ctx = ClientTestContext::setup_reliable(SipMethod::Invite).await;
        let expected_requests = 1;
        let expected_retrans = 0;

        let opt_err = ctx.client.receive_provisional_response().await.err();

        std::assert_matches!(
            opt_err,
            Some(Error::Transaction(TransactionError::Timeout)),
            "Expected Transaction::Timeout, got {opt_err:?}"
        );

        assert_eq!(
            ctx.transport.sent_count(),
            expected_requests + expected_retrans,
            "sent count should match {expected_requests} requests and {expected_retrans} retransmissions"
        );
    }

    #[tokio::test]
    async fn invite_transitions_from_calling_to_proceeding_when_receiving_1xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::Trying).await;

        ctx.client
            .receive_provisional_response()
            .await
            .expect("Error receiving provisional response")
            .expect("Expected provisional response, but received None");

        assert_eq_state!(
            ctx.state,
            State::Proceeding,
            "client INVITE must transition to the Proceeding state when receiving 1xx response"
        );
    }

    #[tokio::test]
    async fn invite_transitions_from_calling_to_completed_when_receiving_3xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::MovedPermanently).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "client INVITE must transition to the Completed state when receiving 3xx response"
        );
    }

    #[tokio::test]
    async fn invite_transitions_from_calling_to_completed_when_receiving_4xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::NotFound).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "client INVITE must transition to the Completed state when receiving 4xx response"
        );
    }

    #[tokio::test]
    async fn invite_transitions_from_calling_to_completed_when_receiving_5xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::ServerTimeout).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "client INVITE must transition to the Completed state when receiving 5xx response"
        );
    }

    #[tokio::test]
    async fn invite_transitions_from_calling_to_completed_when_receiving_6xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::Decline).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "client INVITE must transition to the Completed state when receiving 6xx response"
        );
    }

    #[tokio::test]
    async fn invite_transitions_from_calling_to_terminated_when_receiving_2xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::Accepted).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Terminated,
            "client INVITE must transition to the Terminated state when receiving 2xx response"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn invite_transitions_from_calling_to_terminated_when_timer_b_fires() {
        let mut ctx = ClientTestContext::setup(SipMethod::Invite).await;

        let opt_err = ctx.client.receive_provisional_response().await.err();

        std::assert_matches!(
            opt_err,
            Some(Error::Transaction(TransactionError::Timeout)),
            "Expected Transaction::Timeout, got {opt_err:?}"
        );

        assert_eq_state!(
            ctx.state,
            State::Terminated,
            "client INVITE must transition to the Terminated state when timer B fires"
        );
    }

    #[tokio::test]
    async fn invite_should_send_ack_when_receiving_3xx_response_in_calling_state() {
        let ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::MovedPermanently).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        let req = ctx.transport.last_request().expect("A request");
        assert_eq!(
            req.req_line.method,
            SipMethod::Ack,
            "client INVITE must generate an ACK request when receiving 3xx response"
        );
    }

    #[tokio::test]
    async fn invite_should_send_ack_when_receiving_4xx_response_in_calling_state() {
        let ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::NotFound).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        let req = ctx.transport.last_request().expect("A request");
        assert_eq!(
            req.req_line.method,
            SipMethod::Ack,
            "client INVITE must generate an ACK request when receiving 4xx response"
        );
    }

    #[tokio::test]
    async fn invite_should_send_ack_when_receiving_5xx_response_in_calling_state() {
        let ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::ServerTimeout).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        let req = ctx.transport.last_request().expect("A request");
        assert_eq!(
            req.req_line.method,
            SipMethod::Ack,
            "client INVITE must generate an ACK request when receiving 5xx response"
        );
    }

    #[tokio::test]
    async fn invite_should_send_ack_when_receiving_6xx_response_in_calling_state() {
        let ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::Decline).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        let req = ctx.transport.last_request().expect("A request");
        assert_eq!(
            req.req_line.method,
            SipMethod::Ack,
            "client INVITE must generate an ACK request when receiving 6xx response"
        );
    }

    #[tokio::test]
    async fn invite_transitions_from_proceeding_to_completed_when_receiving_3xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::MovedPermanently).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "client INVITE must transition to the Completed state when receiving 3xx response"
        );
    }

    #[tokio::test]
    async fn invite_transitions_from_proceeding_to_completed_when_receiving_4xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::NotFound).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "client INVITE must transition to the Completed state when receiving 4xx response"
        );
    }

    #[tokio::test]
    async fn invite_transitions_from_proceeding_to_completed_when_receiving_5xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::ServerTimeout).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "client INVITE must transition to the Completed state when receiving 5xx response"
        );
    }

    #[tokio::test]
    async fn invite_transitions_from_proceeding_to_completed_when_receiving_6xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::Decline).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "client INVITE must transition to the Completed state when receiving 6xx response"
        );
    }

    #[tokio::test]
    async fn invite_transitions_from_proceeding_to_terminated_when_receiving_2xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::Accepted).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Terminated,
            "client INVITE must transition to the Completed state when receiving receiving 2xx response"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn invite_should_not_retransmit_request_in_proceeding_state() {
        let mut ctx = ClientTestContext::setup(SipMethod::Invite).await;
        let expected_requests = 1;
        let expected_retrans = 0;

        ctx.server.respond(StatusCode::Trying).await;

        ctx.client
            .receive_provisional_response()
            .await
            .expect("Error receiving provisional response")
            .expect("Expected provisional response, but received None");

        assert_eq_state!(
            ctx.state,
            State::Proceeding,
            "client INVITE must transition to the Proceeding state when receiving receiving 1xx response"
        );

        ctx.timer.wait_for_retransmissions(5).await;

        assert_eq!(
            ctx.transport.sent_count(),
            expected_requests + expected_retrans
        );
    }

    #[tokio::test]
    async fn invite_should_send_ack_when_receiving_3xx_response_in_proceeding_state() {
        let mut ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::Trying).await;

        ctx.client
            .receive_provisional_response()
            .await
            .expect("Error receiving provisional response")
            .expect("Expected provisional response, but received None");

        assert_eq_state!(
            ctx.state,
            State::Proceeding,
            "client INVITE must transition to the Proceeding state when receiving receiving 1xx response"
        );

        ctx.server.respond(StatusCode::MovedPermanently).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        let req = ctx.transport.last_request().expect("A request");
        assert_eq!(
            req.req_line.method,
            SipMethod::Ack,
            "client INVITE must generate an ACK request when receiving 3xx response"
        );
    }

    #[tokio::test]
    async fn invite_should_send_ack_after_4xx_response_in_proceeding_state() {
        let mut ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::Trying).await;

        ctx.client
            .receive_provisional_response()
            .await
            .expect("Error receiving provisional response")
            .expect("Expected provisional response, but received None");

        assert_eq_state!(
            ctx.state,
            State::Proceeding,
            "client INVITE must transition to the Proceeding state when receiving receiving 1xx response"
        );

        ctx.server.respond(StatusCode::NotFound).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        let req = ctx.transport.last_request().expect("A request");
        assert_eq!(
            req.req_line.method,
            SipMethod::Ack,
            "client INVITE must generate an ACK request when receiving 4xx response"
        );
    }

    #[tokio::test]
    async fn invite_should_send_ack_when_receiving_5xx_response_in_proceeding_state() {
        let mut ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::Trying).await;

        ctx.client
            .receive_provisional_response()
            .await
            .expect("Error receiving provisional response")
            .expect("Expected provisional response, but received None");

        assert_eq_state!(
            ctx.state,
            State::Proceeding,
            "client INVITE must transition to the Proceeding state when receiving receiving 1xx response"
        );

        ctx.server.respond(StatusCode::ServerTimeout).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        let req = ctx.transport.last_request().expect("A request");
        assert_eq!(
            req.req_line.method,
            SipMethod::Ack,
            "client INVITE must generate an ACK request when receiving 5xx response"
        );
    }

    #[tokio::test]
    async fn invite_should_send_ack_after_6xx_response_in_proceeding_state() {
        let mut ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::Trying).await;

        ctx.client
            .receive_provisional_response()
            .await
            .expect("Error receiving provisional response")
            .expect("Expected provisional response, but received None");

        assert_eq_state!(
            ctx.state,
            State::Proceeding,
            "client INVITE must transition to the Proceeding state when receiving receiving 1xx response"
        );

        ctx.server.respond(StatusCode::Decline).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        let req = ctx.transport.last_request().expect("A request");
        assert_eq!(
            req.req_line.method,
            SipMethod::Ack,
            "client INVITE must generate an ACK request when receiving 6xx response"
        );
    }

    #[tokio::test]
    async fn invite_should_pass_provisional_responses_to_tu_in_proceeding_state() {
        let mut ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::Trying).await;
        ctx.server.respond(StatusCode::Ringing).await;

        let incoming = ctx
            .client
            .receive_provisional_response()
            .await
            .expect("Error receiving provisional response")
            .expect("Expected provisional response, but received None");

        assert_eq!(
            incoming.response.status_line.code,
            StatusCode::Trying,
            "should match 100 status code"
        );

        let incoming = ctx
            .client
            .receive_provisional_response()
            .await
            .expect("Error receiving provisional response")
            .expect("Expected provisional response, but received None");

        assert_eq!(
            incoming.response.status_line.code,
            StatusCode::Ringing,
            "should match 180 status code"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn invite_transitions_from_completed_to_terminated_when_timer_d_fires() {
        let mut ctx = ClientTestContext::setup(SipMethod::Invite).await;

        ctx.server.respond(StatusCode::MovedPermanently).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "client INVITE must transition to the Completed state when receiving 3xx response"
        );

        tokio::time::sleep(T1 * 64).await;

        assert_eq_state!(
            ctx.state,
            State::Terminated,
            "client INVITE must transition to the Terminated state when timer D fires"
        );
    }

    // Non-INVITE Client tests

    #[tokio::test(start_paused = true)]
    async fn non_invite_should_not_start_timer_e_when_transport_is_reliable() {
        let mut ctx = ClientTestContext::setup_reliable(SipMethod::Invite).await;
        let expected_requests = 1;
        let expected_retrans = 0;

        let opt_err = ctx.client.receive_provisional_response().await.err();

        std::assert_matches!(
            opt_err,
            Some(Error::Transaction(TransactionError::Timeout)),
            "Expected Transaction::Timeout, got {opt_err:?}"
        );

        assert_eq!(
            ctx.transport.sent_count(),
            expected_requests + expected_retrans,
            "sent count should match {expected_requests} requests and {expected_retrans} retransmissions"
        );
    }

    #[tokio::test]
    async fn non_invite_transitions_from_trying_to_proceeding_when_receiving_1xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Register).await;

        ctx.server.respond(StatusCode::Trying).await;

        ctx.client
            .receive_provisional_response()
            .await
            .expect("Error receiving provisional response")
            .expect("Expected provisional response, but received None");

        assert_eq_state!(
            ctx.state,
            State::Proceeding,
            "client non-INVITE must transition to the Proceeding state when receiving receiving 1xx response"
        );
    }

    #[tokio::test]
    async fn non_invite_transitions_from_trying_to_completed_when_receiving_2xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Options).await;

        ctx.server.respond(StatusCode::MovedPermanently).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "client non-INVITE must transition to the Completed state when receiving receiving 6xx response"
        );
    }

    #[tokio::test]
    async fn non_invite_transitions_from_trying_to_completed_when_receiving_3xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Options).await;

        ctx.server.respond(StatusCode::MovedPermanently).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "client non-INVITE must transition to the Completed state when receiving receiving 3xx response"
        );
    }

    #[tokio::test]
    async fn non_invite_transitions_from_trying_to_completed_when_receiving_4xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Options).await;

        ctx.server.respond(StatusCode::NotFound).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "client non-INVITE must transition to the Completed state when receiving receiving 4xx response"
        );
    }

    #[tokio::test]
    async fn non_invite_transitions_from_trying_to_completed_when_receiving_5xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Options).await;

        ctx.server.respond(StatusCode::ServerTimeout).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "client non-INVITE must transition to the Completed state when receiving receiving 5xx response"
        );
    }

    #[tokio::test]
    async fn non_invite_transitions_from_trying_to_completed_when_receiving_6xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Options).await;

        ctx.server.respond(StatusCode::Decline).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "client non-INVITE must transition to the Completed state when receiving receiving 6xx response"
        );
    }
    #[tokio::test]
    async fn non_invite_transitions_from_proceeding_to_completed_when_receiving_3xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Options).await;

        ctx.server.respond(StatusCode::MovedPermanently).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "client non-INVITE must transition to the Completed state when receiving receiving 3xx response"
        );
    }

    #[tokio::test]
    async fn non_invite_transitions_from_proceeding_to_completed_when_receiving_4xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Options).await;

        ctx.server.respond(StatusCode::NotFound).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "client non-INVITE must transition to the Completed state when receiving receiving 4xx response"
        );
    }

    #[tokio::test]
    async fn non_invite_transitions_from_proceeding_to_completed_when_receiving_5xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Options).await;

        ctx.server.respond(StatusCode::ServerTimeout).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "client non-INVITE must transition to the Completed state when receiving receiving 5xx response"
        );
    }

    #[tokio::test]
    async fn non_invite_transitions_from_proceeding_to_completed_when_receiving_6xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Options).await;

        ctx.server.respond(StatusCode::Decline).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "client non-INVITE must transition to the Completed state when receiving receiving 6xx response"
        );
    }

    #[tokio::test]
    async fn non_invite_transitions_from_proceeding_to_completed_when_receiving_2xx_response() {
        let mut ctx = ClientTestContext::setup(SipMethod::Options).await;

        ctx.server.respond(StatusCode::Accepted).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "client non-INVITE must transition to the Completed state when receiving receiving 2xx response"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn non_invite_transitions_from_trying_to_terminated_when_timer_f_fires() {
        let mut ctx = ClientTestContext::setup(SipMethod::Register).await;

        let opt_err = ctx.client.receive_provisional_response().await.err();

        std::assert_matches!(
            opt_err,
            Some(Error::Transaction(TransactionError::Timeout)),
            "Expected Transaction::Timeout, got {opt_err:?}"
        );

        assert_eq_state!(
            ctx.state,
            State::Terminated,
            "client non-INVITE must transition to the Terminated state when timer F fires"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn non_invite_should_not_retransmit_request_in_proceeding_state() {
        let mut ctx = ClientTestContext::setup(SipMethod::Options).await;
        let expected_requests = 1;
        let expected_retrans = 0;

        ctx.server.respond(StatusCode::Trying).await;

        ctx.client
            .receive_provisional_response()
            .await
            .expect("Error receiving provisional response")
            .expect("Expected provisional response, but received None");

        assert_eq_state!(
            ctx.state,
            State::Proceeding,
            "client non-INVITE must transition to the Proceeding state when receiving receiving 1xx response"
        );

        ctx.timer.wait_for_retransmissions(5).await;

        assert_eq!(
            ctx.transport.sent_count(),
            expected_requests + expected_retrans,
            "sent count should match {expected_requests} requests and {expected_retrans} retransmissions"
        );
    }

    #[tokio::test]
    async fn non_invite_should_pass_provisional_responses_to_tu_in_proceeding_state() {
        let mut ctx = ClientTestContext::setup(SipMethod::Options).await;

        ctx.server.respond(StatusCode::Trying).await;
        ctx.server.respond(StatusCode::Ringing).await;

        let response = ctx
            .client
            .receive_provisional_response()
            .await
            .expect("Error receiving provisional response")
            .expect("Expected provisional response, but received None");

        assert_eq!(
            response.response.status_line.code,
            StatusCode::Trying,
            "should match 100 status code"
        );

        let response = ctx
            .client
            .receive_provisional_response()
            .await
            .expect("Error receiving provisional response")
            .expect("Expected provisional response, but received None");

        assert_eq!(
            response.response.status_line.code,
            StatusCode::Ringing,
            "should match 180 status code"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn non_invite_transitions_from_completed_to_terminated_when_timer_k_fires() {
        let mut ctx = ClientTestContext::setup(SipMethod::Options).await;

        ctx.server.respond(StatusCode::MovedPermanently).await;

        ctx.client
            .receive_final_response()
            .await
            .expect("Error receiving final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "client non-INVITE must transition to the Completed state when receiving receiving 3xx response"
        );

        tokio::time::sleep(T1 * 64).await;

        assert_eq_state!(
            ctx.state,
            State::Terminated,
            "should transition to Terminated after timer d fires"
        );
    }
}
