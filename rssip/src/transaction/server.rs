use tokio::sync::mpsc::{self};
use tokio::task;
use tokio::time::{self, Instant};

use crate::endpoint::Endpoint;
use crate::error::{Error, Result};
use crate::message::ReasonPhrase;
use crate::message::method::SipMethod;
use crate::message::status_code::StatusCode;
use crate::transaction::fsm::{State, StateMachine};
use crate::transaction::timers::{T1, T2, T4};
use crate::transaction::{TransactionEntry, TransactionKey};
use crate::transport::incoming::{IncomingMessage, IncomingRequest};
use crate::transport::outgoing::{OutgoingDestInfo, OutgoingResponse, TargetTransportInfo};

const CHANNEL_RECV_BUG: &str = "receiver should exist when no provisional response is sent";

/// A Server Transaction.
///
/// Represents a SIP server transaction.
pub struct ServerTransaction {
    transaction_key: TransactionKey,
    endpoint: Endpoint,
    state_machine: StateMachine,
    request: IncomingRequest,
    target_info: Option<TargetTransportInfo>,
    receiver: Option<mpsc::Receiver<IncomingMessage>>,
    provisonal_retrans_handle: Option<ProvisionalRetransHandle>,
}

impl ServerTransaction {
    /// Create a new [`ServerTransaction`] from the given request.
    ///
    /// # Panics
    ///
    /// Panics if request method is `ACK`.
    pub fn from_request(request: IncomingRequest, endpoint: Endpoint) -> Self {
        let method = request.req_line.method;
        assert_ne!(
            method,
            SipMethod::Ack,
            "ACK requests do not create transactions"
        );

        let initial_state = if method == SipMethod::Invite {
            State::Proceeding
        } else {
            State::Trying
        };
        let state_machine = StateMachine::new(initial_state);
        let (sender, receiver) = mpsc::channel(10);

        let transaction_key = TransactionKey::from_request(&request);

        endpoint
            .tsx_plugin()
            .add_transaction(transaction_key.clone(), sender);

        let server_tsx = Self {
            endpoint,
            transaction_key,
            request,
            state_machine,
            receiver: Some(receiver),
            target_info: None,
            provisonal_retrans_handle: None,
        };

        log::trace!(
            "Server transaction created [{:#?}] ({})",
            method,
            server_tsx
                .request
                .incoming_info
                .mandatory_headers
                .call_id
                .id()
        );

        server_tsx
    }

    /// Sends a provisional response with the given `status`.
    ///
    /// This is a shortcut for:
    ///
    /// ```no_run
    /// let response = transaction.create_response(status, None);
    /// transaction.send_provisional_response(response).await;
    /// ```
    /// See [`send_provisional_response`](Self::send_provisional_response) for more info.
    pub async fn send_provisional_status(&mut self, status: StatusCode) -> Result<()> {
        let response = self.create_response(status, None);

        self.send_provisional_response(response).await?;

        Ok(())
    }

    /// Sends a provisional response.
    ///
    /// # Panics
    ///
    /// Panics if the `response` is not provisional (`1xx`).
    pub async fn send_provisional_response(
        &mut self,
        mut response: OutgoingResponse,
    ) -> Result<()> {
        let code = response.status_line.code;

        if !code.is_provisional() {
            return Err(Error::Custom(format!(
                "Invalid provisional response (expected 1xx) got {code:?}"
            )));
        }

        self.send_response(&mut response).await?;

        if let Some(ref mut handle) = self.provisonal_retrans_handle {
            handle
                .provisional_tx
                .send(response)
                .map_err(|_| Error::ChannelClosed)?
        } else {
            let handle = self.spawn_retransmit_provisional_task(response);
            self.provisonal_retrans_handle = Some(handle);
        }

        Ok(())
    }

    /// Sends a final response with the given `status`.
    ///
    /// This is a shortcut for:
    ///
    /// ```no_run
    /// let response = transaction.create_response(status, None);
    /// transaction.send_final_response(response).await;
    /// ```
    /// See [`send_final_response`](Self::send_final_response) for more info.
    pub async fn send_final_status(self, status: StatusCode) -> Result<()> {
        let response = self.create_response(status, None);

        self.send_final_response(response).await?;

        Ok(())
    }

    /// Sends a final response.
    ///
    /// # Panics
    ///
    /// Panics if the `response` is not final (`2xx-6xx`).
    pub async fn send_final_response(mut self, mut response: OutgoingResponse) -> Result<()> {
        let code = response.status_line.code;

        if !code.is_final() {
            return Err(Error::Custom(format!(
                "Invalid final response (expected 2xx-6xx) got {code:?}"
            )));
        }

        let is_invite_tsx = self.is_invite_tsx();

        self.send_response(&mut response).await?;

        // INVITE - 2xx from TU send response --> Terminated
        if is_invite_tsx && matches!(code.as_u16(), 200..299) {
            self.set_state(State::Terminated);
            return Ok(());
        }
        // INVITE - 300-699 from TU send response --> Completed
        // non-INVITE - 200-699 from TU send response --> Completed
        self.set_state(State::Completed);

        // INVITE/non-INVITE - reliable --> timer_k/timer_j fire in zero seconds.
        if self.is_reliable() {
            self.set_state(State::Terminated);
            return Ok(());
        }
        // timer_k/timer_j
        let deadline = Instant::now() + if is_invite_tsx { T4 } else { T1 * 64 };

        let mut channel = if let Some(task) = self.provisonal_retrans_handle.take() {
            task.join_handle.await.map_err(std::io::Error::from)?
        } else {
            self.receiver.take().expect(CHANNEL_RECV_BUG)
        };

        if is_invite_tsx && !self.is_reliable() {
            tokio::spawn(async move {
                let mut retrans_interval = T1;
                loop {
                    let fut = time::timeout(retrans_interval, channel.recv());

                    match time::timeout_at(deadline, fut).await {
                        Ok(Ok(Some(IncomingMessage::Request(req))))
                            if req.req_line.method == SipMethod::Ack =>
                        {
                            self.set_state(State::Confirmed);
                            time::sleep(T4).await;
                            self.set_state(State::Terminated);
                            break;
                        }
                        Ok(Ok(Some(_msg))) => {
                            _ = self.send_response(&mut response).await;
                            continue;
                        }
                        Ok(Err(_elapsed)) => {
                            // retransmit
                            _ = self.send_response(&mut response).await;
                            retrans_interval = std::cmp::min(retrans_interval * 2, T2);
                            continue;
                        }
                        Err(_elapsed) => {
                            self.set_state(State::Terminated);
                            break;
                        }
                        Ok(Ok(None)) => break,
                    }
                }
            });
        } else {
            tokio::spawn(async move {
                while let Ok(Some(_)) = time::timeout_at(deadline, channel.recv()).await {
                    if let Err(err) = self.send_response(&mut response).await {
                        log::error!("Failed to send response: {}", err);
                    }
                }
                self.set_state(State::Terminated);
            });
        }

        Ok(())
    }

    pub fn create_response(
        &self,
        code: StatusCode,
        reason: Option<ReasonPhrase>,
    ) -> OutgoingResponse {
        if let Some(target) = self.target_info.clone() {
            let source_addr = target.transport.local_addr();
            let response = self.endpoint.create_response(&self.request, code, reason);

            let dest_info = OutgoingDestInfo {
                host_port: (source_addr.into(), target.transport.protocol()),
                transport: Some(target),
            };

            OutgoingResponse {
                response,
                dest_info,
                encoded: Default::default(),
            }
        } else {
            self.endpoint
                .create_outgoing_response(&self.request, code, reason)
        }
    }

    pub(crate) fn request(&self) -> &IncomingRequest {
        &self.request
    }

    pub(crate) fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub fn is_invite_tsx(&self) -> bool {
        self.request.req_line.method == SipMethod::Invite
    }

    pub(crate) fn key(&self) -> &TransactionKey {
        &self.transaction_key
    }

    pub(crate) fn get_entry(&self) -> TransactionEntry {
        self.endpoint
            .tsx_plugin()
            .get_entry(self.key())
            .expect("must exists while server tsx is not dropped")
    }

    fn set_state(&mut self, state: State) {
        self.state_machine.set_state(state);
    }

    pub fn state_machine_mut(&mut self) -> &mut StateMachine {
        &mut self.state_machine
    }

    async fn send_response(&mut self, response: &mut OutgoingResponse) -> Result<()> {
        self.endpoint.send_outgoing_response(response).await?;

        if self.target_info.is_none() {
            self.target_info = response.dest_info.transport.clone();
        }
        Ok(())
    }

    fn is_reliable(&self) -> bool {
        self.request
            .incoming_info
            .transport_msg
            .transport
            .is_reliable()
    }

    fn spawn_retransmit_provisional_task(
        &mut self,
        mut response: OutgoingResponse,
    ) -> ProvisionalRetransHandle {
        let mut receiver = self.receiver.take().expect(CHANNEL_RECV_BUG);

        self.set_state(State::Proceeding);

        let mut state_rx = self.state_machine.subscribe_state();
        let (provisional_tx, mut tu_provisional_rx) = mpsc::unbounded_channel();

        let endpoint_clone = self.endpoint.clone();
        let join_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;

                    _= state_rx.recv() => {
                        log::debug!("Leaving Proceding State...");
                        return receiver;
                    }
                    Some(new_tu_provisional) = tu_provisional_rx.recv() => {
                        response = new_tu_provisional;
                    }
                    Some(_msg) = receiver.recv() => {
                           if let Err(err) = endpoint_clone.send_outgoing_response(&mut response)
                           .await {
                            log::error!("Failed to retransmit: {}", err);
                           }
                    }
                }
            }
        });

        ProvisionalRetransHandle {
            provisional_tx,
            join_handle,
        }
    }
}

struct ProvisionalRetransHandle {
    join_handle: task::JoinHandle<mpsc::Receiver<IncomingMessage>>,
    provisional_tx: mpsc::UnboundedSender<OutgoingResponse>,
}

impl Drop for ServerTransaction {
    fn drop(&mut self) {
        self.endpoint
            .tsx_plugin()
            .remove_transaction(&self.transaction_key);
        log::trace!(
            "Server transaction destroyed [{:#?}] ({})",
            self.request.req_line.method,
            self.request.incoming_info.mandatory_headers.call_id.id()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macros::assert_eq_tsx_state as assert_eq_state;
    use crate::message::status_code::StatusCode;
    use crate::test_utils;
    use crate::transaction::fsm;
    use crate::transaction::timers::MockRetransTimer;
    use crate::transport::incoming::IncomingRequest;
    use crate::transport::{MockTransport, TransportHandle};

    struct ServerTestContext {
        server: ServerTransaction,
        client: MockClientTsx,
        transport: MockTransport,
        timer: MockRetransTimer,
        state: fsm::TsxStateChangeReceiver,
    }

    struct MockClientTsx {
        sender: mpsc::Sender<IncomingMessage>,
        request: IncomingRequest,
    }

    impl ServerTestContext {
        pub async fn setup(method: SipMethod) -> Self {
            Self::new(method, MockTransport::new_udp()).await
        }

        pub async fn setup_reliable(method: SipMethod) -> Self {
            Self::new(method, MockTransport::new_tcp()).await
        }

        async fn new(method: SipMethod, transport: MockTransport) -> Self {
            let transport_impl = TransportHandle::new(transport.clone());

            let endpoint = test_utils::create_test_endpoint().await;
            let request = test_utils::create_test_request(method, transport_impl);

            let mut server = ServerTransaction::from_request(request.clone(), endpoint.clone());

            let sender = endpoint.tsx_plugin().get_entry(server.key()).unwrap();
            let client = MockClientTsx { sender, request };
            let timer = MockRetransTimer::new();
            let state = server.state_machine_mut().subscribe_state();

            Self {
                server,
                client,
                transport,
                timer,
                state,
            }
        }
    }

    impl MockClientTsx {
        async fn retransmit_n_times(&self, n: usize) {
            for _ in 0..n {
                self.retransmit().await;
            }
        }

        async fn retransmit(&self) {
            self.send(self.request.clone()).await;
        }

        async fn send_ack_request(&mut self) {
            let mut incoming = self.request.clone();
            incoming.request.req_line.method = SipMethod::Ack;
            self.send(incoming).await;
        }

        async fn send(&self, request: IncomingRequest) {
            self.sender
                .send(IncomingMessage::Request(request))
                .await
                .unwrap();
            tokio::task::yield_now().await;
        }
    }

    // INVITE Server tests

    #[tokio::test]
    async fn invite_transitions_to_proceeding_when_created_from_request() {
        let ctx = ServerTestContext::setup(SipMethod::Invite).await;

        assert_eq!(
            ctx.server.state_machine.state(),
            State::Proceeding,
            "server INVITE must transition to the Proceeding state when constructed for a request"
        );
    }
    #[tokio::test]
    async fn invite_transitions_to_confirmed_when_receiving_ack() {
        let mut ctx = ServerTestContext::setup(SipMethod::Invite).await;

        ctx.server
            .send_final_status(StatusCode::MovedPermanently)
            .await
            .expect("Error sending final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "server INVITE must transition to the Completed state when sending 300-699 response"
        );

        ctx.client.send_ack_request().await;

        assert_eq_state!(
            ctx.state,
            State::Confirmed,
            "server INVITE must transition to the Confirmed state when receiving the ACK request"
        );
    }

    #[tokio::test]
    async fn invite_unreliable_transitions_to_terminated_when_sending_2xx_response() {
        let mut ctx = ServerTestContext::setup(SipMethod::Invite).await;

        ctx.server
            .send_final_status(StatusCode::Accepted)
            .await
            .expect("Error sending final response");

        assert_eq_state!(
            ctx.state,
            State::Terminated,
            "server INVITE must transition to the Terminated state when sending 2xx response"
        );
    }

    #[tokio::test]
    async fn invite_reliable_transitions_to_terminated_when_sending_2xx_response() {
        let mut ctx = ServerTestContext::setup_reliable(SipMethod::Invite).await;

        ctx.server
            .send_final_status(StatusCode::Accepted)
            .await
            .expect("Error sending final response");

        assert_eq_state!(
            ctx.state,
            State::Terminated,
            "server INVITE must transition to the Terminated state when sending 2xx response"
        );
    }

    #[tokio::test]
    async fn invite_should_retransmit_response_when_receiving_request_retransmission() {
        let ctx = ServerTestContext::setup(SipMethod::Invite).await;
        let expected_responses = 1;
        let expected_retrans = 3;

        ctx.server
            .send_final_status(StatusCode::MovedPermanently)
            .await
            .expect("Error sending final response");

        ctx.client.retransmit_n_times(expected_retrans).await;

        assert_eq!(
            ctx.transport.sent_count(),
            expected_responses + expected_retrans,
            "sent count should match {expected_responses} responses and {expected_retrans} retransmissions"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn invite_must_cease_retransmission_when_receiving_ack() {
        let mut ctx = ServerTestContext::setup(SipMethod::Invite).await;
        let expected_responses = 1;
        let expected_retrans = 2;

        ctx.server
            .send_final_status(StatusCode::MovedPermanently)
            .await
            .expect("Error sending final response");

        ctx.timer.wait_for_retransmissions(expected_retrans).await;

        ctx.client.send_ack_request().await;

        // Should not retransmit at this point.
        ctx.timer.wait_for_retransmissions(3).await;

        assert_eq!(
            ctx.transport.sent_count(),
            expected_responses + expected_retrans,
            "sent count should match {expected_responses} responses and {expected_retrans} retransmissions"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn invite_timer_h_must_be_set_for_reliable_transports() {
        let mut ctx = ServerTestContext::setup_reliable(SipMethod::Invite).await;

        ctx.server
            .send_final_status(StatusCode::MovedPermanently)
            .await
            .expect("Error sending final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "server INVITE must transition to the Completed state when sending final 200-699 response"
        );

        tokio::time::sleep(T1 * 64).await;

        assert_eq_state!(
            ctx.state,
            State::Terminated,
            "server INVITE must transition to the Terminated state when timer H fires"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn invite_timer_h_must_be_set_for_unreliable_transports() {
        let mut ctx = ServerTestContext::setup(SipMethod::Invite).await;

        ctx.server
            .send_final_status(StatusCode::MovedPermanently)
            .await
            .expect("Error sending final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "server INVITE must transition to the Completed state when sending 200-699 response"
        );

        tokio::time::sleep(T1 * 64).await;

        assert_eq_state!(
            ctx.state,
            State::Terminated,
            "server INVITE must transition to the Terminated state when timer H fires"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn invite_transitions_to_terminated_when_timer_i_fires() {
        let mut ctx = ServerTestContext::setup(SipMethod::Invite).await;

        ctx.server
            .send_final_status(StatusCode::MovedPermanently)
            .await
            .expect("Error sending final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "server INVITE must must transition to the Completed when sending 300-699 response"
        );

        ctx.client.send_ack_request().await;

        assert_eq_state!(
            ctx.state,
            State::Confirmed,
            "server INVITE must transition to the Confirmed state when receiving ACK request"
        );

        tokio::time::sleep(T4).await;

        assert_eq_state!(
            ctx.state,
            State::Terminated,
            "server INVITE must transition to the Terminated state when timer I fires"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn invite_retransmit_response_when_timer_g_fires() {
        let mut ctx = ServerTestContext::setup(SipMethod::Invite).await;
        let expected_responses = 1;
        let expected_retrans = 3;

        ctx.server
            .send_final_status(StatusCode::MovedPermanently)
            .await
            .expect("Error sending final response");

        ctx.timer.wait_for_retransmissions(expected_retrans).await;

        assert_eq!(
            ctx.transport.sent_count(),
            expected_responses + expected_retrans,
            "sent count should match {expected_responses} requests and {expected_retrans} retransmissions"
        );
    }

    // Non-INVITE Server tests

    #[tokio::test]
    async fn non_invite_transitions_to_trying_when_created_from_request() {
        let ctx = ServerTestContext::setup(SipMethod::Options).await;

        assert_eq!(
            ctx.server.state_machine.state(),
            State::Trying,
            "server non-INVITE must transition to the Trying state when constructed for a request"
        );
    }

    #[tokio::test]
    async fn non_invite_transition_to_proceeding_when_sending_1xx_response() {
        let mut ctx = ServerTestContext::setup(SipMethod::Options).await;

        ctx.server
            .send_provisional_status(StatusCode::Trying)
            .await
            .expect("Error sending provisional response");

        assert_eq_state!(
            ctx.state,
            State::Proceeding,
            "server non-INVITE must transition to the Proceeding state when sending 1xx response"
        );
    }

    #[tokio::test]
    async fn non_invite_transition_to_completed_when_sending_non_2xx_response() {
        let mut ctx = ServerTestContext::setup(SipMethod::Options).await;

        ctx.server
            .send_final_status(StatusCode::ServerTimeout)
            .await
            .expect("Error sending final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "server non-INVITE must transition to the Completed state when sending 200-699 response"
        );
    }

    #[tokio::test]
    async fn non_invite_reliable_transition_to_terminated_when_sending_2xx_response() {
        let mut ctx = ServerTestContext::setup_reliable(SipMethod::Options).await;

        ctx.server
            .send_final_status(StatusCode::Accepted)
            .await
            .expect("Error sending final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "server non-INVITE must transition to the Completed when sending 200-699 response"
        );

        assert_eq_state!(
            ctx.state,
            State::Terminated,
            "server non-INVITE must transition to the Terminated state when sending 2xx response"
        );
    }

    #[tokio::test]
    async fn non_invite_reliable_transition_to_terminated_when_sending_non_2xx_response() {
        let mut ctx = ServerTestContext::setup_reliable(SipMethod::Options).await;

        ctx.server
            .send_final_status(StatusCode::ServerTimeout)
            .await
            .expect("Error sending final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "server non-INVITE must transition to the Completed when sending 200-699 response"
        );

        assert_eq_state!(
            ctx.state,
            State::Terminated,
            "server non-INVITE must transition to the Terminated state when sending 2xx response"
        );
    }

    #[tokio::test]
    async fn non_invite_absorbs_retransmission_in_trying_state() {
        let ctx = ServerTestContext::setup(SipMethod::Options).await;
        let expected_retrans = 0;

        ctx.client.retransmit_n_times(2).await;

        assert_eq!(
            ctx.transport.sent_count(),
            expected_retrans,
            "sent count should match {expected_retrans} retransmissions"
        );
    }

    #[tokio::test]
    async fn non_invite_retransmit_provisional_response_when_receiving_request_retransmission() {
        let mut ctx = ServerTestContext::setup(SipMethod::Options).await;
        let expected_responses = 1;
        let expected_retrans = 4;

        ctx.server
            .send_provisional_status(StatusCode::Trying)
            .await
            .expect("Error sending provisional response");

        ctx.client.retransmit_n_times(expected_retrans).await;

        assert_eq!(
            ctx.transport.sent_count(),
            expected_responses + expected_retrans,
            "sent count should match {expected_responses} responses and {expected_retrans} retransmissions"
        );
    }

    #[tokio::test]
    async fn non_invite_retransmit_final_response_when_receiving_request_retransmission() {
        let ctx = ServerTestContext::setup(SipMethod::Register).await;
        let expected_responses = 1;
        let expected_retrans = 2;

        ctx.server
            .send_final_status(StatusCode::Accepted)
            .await
            .expect("Error sending final response");

        ctx.client.retransmit_n_times(expected_retrans).await;

        assert_eq!(
            ctx.transport.sent_count(),
            expected_responses + expected_retrans,
            "sent count should match {expected_responses} responses and {expected_retrans} retransmissions"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn non_invite_transitions_to_terminated_when_timer_j_fires() {
        let mut ctx = ServerTestContext::setup(SipMethod::Bye).await;

        ctx.server
            .send_final_status(StatusCode::Accepted)
            .await
            .expect("Error sending final response");

        assert_eq_state!(
            ctx.state,
            State::Completed,
            "server non-INVITE must must transition to the Completed state when sending 200-699 response"
        );

        tokio::time::sleep(T1 * 64).await;

        assert_eq_state!(
            ctx.state,
            State::Terminated,
            "server non-INVITE must transition to the Terminated state when timer J fires"
        );
    }
}
