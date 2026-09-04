use std::net::SocketAddr;

use tokio::{net::UdpSocket, sync::mpsc};

use crate::rtp::packet::RtpPacket;


pub struct RtpSession {
    // Local RTP address
    local_addr: SocketAddr,

    stream: RtpStream
}

struct RtpStream {
    sender: mpsc::UnboundedSender<RtpPacket>,
    receiver: mpsc::UnboundedReceiver<RtpPacket>,
}

impl RtpSession {
    pub async fn create(sock: SocketAddr) -> std::io::Result<Self> {
        let rtp_sock = UdpSocket::bind(sock).await?;

        let (tx, rx) = mpsc::unbounded_channel::<RtpPacket>();

        todo!()
    }
}