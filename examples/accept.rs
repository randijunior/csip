use std::error::Error;
use std::net::IpAddr;

use rssip::IncomingRequest;
use rssip::endpoint::{self, Endpoint, ToTake};
use rssip::media::codec::Codec;
use rssip::media::media::MediaParams;
use rssip::media::sdp::Direction;
use rssip::message::SipBody;
use rssip::message::headers::Contact;
use rssip::message::method::SipMethod;
use rssip::message::status_code::StatusCode;
use rssip::sip_ua::dialog::DialogPlugin;
use rssip::sip_ua::session::{Established, MediaEvent, Session, SessionEvent};
use rssip::transaction::TsxPlugin;
use tracing::Level;
use tracing_subscriber::fmt::time::ChronoLocal;

struct Logger;

#[async_trait::async_trait]
impl endpoint::Plugin for Logger {
    fn name(&self) -> &'static str {
        "logger"
    }

    async fn on_outgoing_response(&self, res: &mut rssip::OutgoingResponse) {
        println!(
            "{}{}{}",
            res.status_line,
            res.headers,
            get_body_utf8(&res.body)
        );
    }
    async fn on_incoming_request(&self, req: ToTake<'_, IncomingRequest>, _endpoint: &Endpoint) {
        println!(
            "{}{}{}",
            req.req_line,
            req.headers,
            get_body_utf8(&req.body)
        );
    }
}

pub struct Acceptor {
    contact: Contact,
}

#[async_trait::async_trait]
impl endpoint::Plugin for Acceptor {
    fn name(&self) -> &'static str {
        "acceptor"
    }

    async fn on_incoming_request(&self, mut req: ToTake<'_, IncomingRequest>, endpoint: &Endpoint) {
        let request = if req.req_line.method == SipMethod::Invite {
            req.take()
        } else {
            return;
        };
        let mut session =
            Session::from_invite(request, self.contact.clone(), endpoint.clone()).unwrap();

        session.progress(StatusCode::Trying, None).await.unwrap();

        let media = MediaParams::audio(IpAddr::from([127, 0, 0, 1]), 17258, Direction::SendRecv)
            .with_codecs(vec![Codec::ULAW, Codec::ALAW, Codec::TELEPHONE_EVENT]);

        session.set_media_params(media);

        let session = session.accept(StatusCode::Ok, None).await.unwrap();

        session_evt_loop(session).await;

        println!("Session ENDED");
    }
}

async fn session_evt_loop(mut session: Session<Established>) {
    while let Some(evt) = session.next_event().await {
        match evt {
            SessionEvent::Terminated(cause) => {
                println!("Terminated, cause = {cause:#?}");
                break;
            }
            SessionEvent::ReInvite(_invite) => println!("Reinvite"),
            SessionEvent::Options(_options) => todo!(),
            SessionEvent::Media(MediaEvent::RtpReceived(_rtp_packet)) => todo!(),
        }
    }
}

fn get_body_utf8(body: &Option<SipBody>) -> &str {
    body.as_ref()
        .map(|b| std::str::from_utf8(&b).unwrap())
        .unwrap_or("")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_max_level(Level::TRACE)
        .with_env_filter("rssip=trace")
        .with_timer(ChronoLocal::new(String::from("%H:%M:%S%.3f")))
        .init();

    let _endpoint = Endpoint::builder()
        .with_plugin(Logger)
        .with_plugin(DialogPlugin::default())
        .with_plugin(TsxPlugin::default())
        .with_plugin(Acceptor {
            contact: "<sip:0.0.0.0:8089>".parse().unwrap(),
        })
        .with_udp_addr("0.0.0.0:8089")
        .build()
        .await?;

    tokio::signal::ctrl_c().await?;
    println!("shutting down");

    Ok(())
}
