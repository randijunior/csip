use crate::codec::Codec;
use crate::sdp;

pub struct MediaSession {
    session_id: u64,
    session_version: u64,

    codecs: Codecs,

    dir: sdp::Direction, // local_addr: IpAddr
                              // remote_addr: IpAddr
}

#[derive(Default)]
pub struct Codecs {
    media_type: sdp::MediaType,
    codecs: Vec<Codec>,
}

impl Codecs {
    pub fn push(&mut self, codec: Codec) {
        self.codecs.push(codec);
    }
}
