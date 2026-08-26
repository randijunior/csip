#![warn(clippy::undocumented_unsafe_blocks)]

//! # rssip
//!
//! A rust library that implements the SIP protocol.
//!

mod core;
pub(crate) mod error;
pub mod macros;
pub mod message;
pub(crate) mod parser;
pub mod sip_ua;
pub mod transaction;
pub mod transport;

pub use core::endpoint::Endpoint;
pub use core::{endpoint, resolver};

pub use error::Result;
pub use media;
pub use transport::incoming::{IncomingMessage, IncomingRequest, IncomingResponse};
pub use transport::outgoing::{OutgoingRequest, OutgoingResponse};
pub mod utils {
    pub use utils::local_ip;

    pub use crate::generate_tag;
}

use std::str;

use error::Error;

/// Branch parameter prefix defined in RFC3261.
pub(crate) const RFC3261_BRANCH_ID: &str = "z9hG4bK";

use rand::distr::{Alphanumeric, SampleString};

pub(crate) fn generate_branch() -> String {
    generate_branch_n(8)
}

pub(crate) fn generate_branch_n(n: usize) -> String {
    let mut branch = String::with_capacity(RFC3261_BRANCH_ID.len() + n);
    branch.push_str(RFC3261_BRANCH_ID);
    Alphanumeric.append_string(&mut rand::rng(), &mut branch, n);
    branch
}

pub fn generate_tag() -> String {
    random_str(8)
}

pub(crate) fn random_str(n: usize) -> String {
    Alphanumeric.sample_string(&mut rand::rng(), n)
}

#[must_use]
pub(crate) fn is_valid_port(v: u16) -> bool {
    matches!(v, 0..=65535)
}

/// Test utilities
#[cfg(test)]
mod test_utils {

    use std::str::FromStr;

    use bytes::Bytes;

    use crate::endpoint::{Endpoint, EndpointBuilder};
    use crate::message::Request;
    use crate::message::headers::{CSeq, CallId, From, Header, Headers, MaxForwards, To, Via};
    use crate::message::method::SipMethod;
    use crate::message::uri::Uri;
    use crate::sip_ua::dialog::DialogPlugin;
    use crate::transaction::TsxPlugin;
    use crate::transport::incoming::{IncomingInfo, IncomingRequest, MandatoryHeaders};
    use crate::transport::{Packet, TransportHandle, TransportMessage};

    pub async fn create_test_endpoint() -> Endpoint {
        EndpointBuilder::new()
            .with_udp_addr("127.0.0.1:0")
            .with_plugin(DialogPlugin::default())
            .with_plugin(TsxPlugin::default())
            .build()
            .await
            .unwrap()
    }

    fn create_test_headers(method: SipMethod) -> Headers {
        let branch = crate::generate_branch();

        let via = Via::from_str(&format!(
            "SIP/2.0/UDP localhost:5060;branch={branch};received=127.0.0.1"
        ))
        .unwrap();
        let from = From::from_str("Alice <sip:alice@localhost>;tag=1928301774").unwrap();
        let to = To::from_str("Bob <sip:bob@localhost>").unwrap();
        let cid = CallId::from("a84b4c76e66710@pc33.atlanta.com");
        let mfowards = MaxForwards::new(70);
        let cseq = CSeq::new(1, method);

        crate::headers! {
            Header::Via(via),
            Header::From(from),
            Header::To(to),
            Header::CallId(cid),
            Header::CSeq(cseq),
            Header::MaxForwards(mfowards)
        }
    }

    pub fn create_test_request(method: SipMethod, transport: TransportHandle) -> IncomingRequest {
        let headers = create_test_headers(method);
        let target = format!("sip:{}", transport.local_addr());
        let uri = Uri::from_str(&target).unwrap();

        let mandatory_headers = MandatoryHeaders::from_headers(&headers).unwrap();

        let request = Request::with_headers(method, uri, headers);
        let packet = Packet::new(Bytes::new(), transport.local_addr());

        let transport_msg = TransportMessage { packet, transport };

        let incoming_info = IncomingInfo {
            transport_msg,
            mandatory_headers,
        };

        IncomingRequest {
            request,
            incoming_info: Box::new(incoming_info),
        }
    }
}
