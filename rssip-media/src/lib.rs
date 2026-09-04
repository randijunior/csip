use std::net::SocketAddr;

use tokio::sync::mpsc;

use crate::{rtp::{packet::RtpPacket}, sdp::SessionDescription};

pub mod codec;
pub mod error;
pub mod negotiator;
pub mod rtp;
pub mod sdp;

pub enum MediaEvent {
    RtpPacket(RtpPacket),
}

pub struct SessionMedia {
   
}

impl SessionMedia {
    pub async fn setup(sdp: &SessionDescription) -> std::io::Result<Self> { todo!() }
    pub async fn receive_event(&mut self) -> std::io::Result<MediaEvent> { todo!() }
}
