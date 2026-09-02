use crate::rtp::header::RtpHeader;

pub struct RtpPacket {
    header: RtpHeader,
    payload: bytes::Bytes,
}
