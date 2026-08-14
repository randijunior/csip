use std::error::Error;

use rssip::IncomingRequest;
use rssip::endpoint::{self, Endpoint, ToTake};
use rssip::message::method::SipMethod;
use rssip::message::status_code::StatusCode;
use tracing::Level;
use tracing_subscriber::fmt::time::ChronoLocal;

pub struct SipStateless;

#[async_trait::async_trait]
impl endpoint::Plugin for SipStateless {
    fn name(&self) -> &'static str {
        "sip-stateless"
    }

    async fn on_incoming_request(&self, mut req: ToTake<'_, IncomingRequest>, endpoint: &Endpoint) {
        if req.req_line.method != SipMethod::Ack {
            let request = req.take();

            let mut response =
                endpoint.create_outgoing_response(&request, StatusCode::NotImplemented, None);

            endpoint
                .send_outgoing_response(&mut response)
                .await
                .unwrap();
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_max_level(Level::TRACE)
        .with_env_filter("rssip=trace")
        .with_timer(ChronoLocal::new(String::from("%H:%M:%S%.3f")))
        .init();

    let _endpoint = Endpoint::builder()
        .with_plugin(SipStateless)
        .with_udp_addr("0.0.0.0:8089")
        .build()
        .await?;

    tokio::signal::ctrl_c().await?;
    println!("shutting down");

    Ok(())
}
