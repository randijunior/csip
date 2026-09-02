use std::net::{IpAddr, SocketAddr};

use crate::codec::Codec;
use crate::sdp::{Direction, MediaType, SdpTransport, SessionDescription};

pub struct MediaSession {
    session_name: String,
    session_id: u64,
    session_version: u64,
    session_onwer: String,

    local_addr: IpAddr,
    remote_addr: Option<IpAddr>,

    direction: Direction,
}

pub struct RtpSession {}

impl RtpSession {
    pub async fn setup(addr: SocketAddr) -> std::io::Result<Self> {
        todo!()
    }
}
