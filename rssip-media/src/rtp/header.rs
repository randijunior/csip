pub struct RtpHeader {
    version: u8,
    padding: bool,
    extension: bool,
    marker: bool,
    sequence_number: u16,
    payload_type: u8,
    ssrc: u32,
    csrc: Vec<u32>,
    timestamp: u32
}