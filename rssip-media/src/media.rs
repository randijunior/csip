use std::net::{IpAddr, SocketAddr};

use crate::codec::Codec;
use crate::sdp::{Direction, MediaType, SdpTransport};


pub struct MediaSession {
    session_name: String,
    session_id: u64,
    session_version: u64,
    session_onwer: String,

    local_addr: IpAddr,
    remote_addr: Option<IpAddr>,

    direction: Direction,

    // RtpSession
}

pub struct MediaParams {
    pub(crate) media_type: MediaType,
    pub(crate)direction: Direction,
    pub(crate)origin_ip: IpAddr,
    pub(crate)transport: SdpTransport,
    pub(crate)codecs: Vec<Codec>,
    pub(crate) port: u16,
}

impl MediaSession {
    pub async fn from_media_params(media_params: &MediaParams)-> std::io::Result<Self> {
           let ip = media_params.ip();
                let port = media_params.port();

                let addr = std::net::SocketAddr::new(ip, port);
                let udp_sock = tokio::net::UdpSocket::bind(addr).await?;
        todo!()
    }
}
