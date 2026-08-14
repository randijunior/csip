use crate::codec::Codec;
use crate::error::{Error, Result};
use crate::media::Codecs;
use crate::sdp::{Attribute, MediaDescription, MediaType, SdpTransport, SessionDescription};

// RFC 3264: An Offer/Answer Model with the Session Description Protocol (SDP)
#[derive(Default)]
pub struct SdpNegotiator {
    remote_offer: Option<SessionDescription>,
    local_offer: Option<SessionDescription>,
    answer: Option<SessionDescription>,
    state: NegotiatorState,
}

#[derive(Default, Debug, PartialEq, Eq)]
enum NegotiatorState {
    #[default]
    Stable,
    RemoteOffer,
    LocalOffer,
    Ready,
    Done,
}

pub struct MediaStream {
    media_type: MediaType,
    codecs: Codecs,
    transport: SdpTransport,
    port: u16,
}

#[derive(Default)]
pub struct MediaStreamBuilder {
    media_type: Option<MediaType>,
    codecs: Codecs,
    transport: Option<SdpTransport>,
    port: Option<u16>,
}

impl SdpNegotiator {
    pub fn with_local(local: SessionDescription) -> Self {
        Self {
            local_offer: Some(local),
            state: NegotiatorState::LocalOffer,
            answer: None,
            remote_offer: None,
        }
    }

    pub fn with_remote(remote: SessionDescription) -> Self {
        Self {
            remote_offer: Some(remote),
            state: NegotiatorState::RemoteOffer,
            answer: None,
            local_offer: None,
        }
    }

    pub fn set_remote_sdp(&mut self, remote: SessionDescription) -> Result<()> {
        self.state = match self.state {
            NegotiatorState::Stable => NegotiatorState::RemoteOffer,
            NegotiatorState::LocalOffer => NegotiatorState::Ready,
            _ => return Err(Error::ErrInvalidNegoState),
        };
        self.remote_offer = Some(remote);
        Ok(())
    }

    pub fn set_local_sdp(&mut self, local: SessionDescription) -> Result<()> {
        self.state = match self.state {
            NegotiatorState::Stable => NegotiatorState::LocalOffer,
            NegotiatorState::RemoteOffer => NegotiatorState::Ready,
            _ => return Err(Error::ErrInvalidNegoState),
        };
        self.local_offer = Some(local);
        Ok(())
    }

    pub fn create_answer(&mut self) -> Result<&SessionDescription> {
        if self.state != NegotiatorState::Ready {
            return Err(Error::ErrInvalidNegoState);
        };

        let remote_offer = self.remote_offer.as_ref().expect("a remote offer");
        let local_offer = self.local_offer.as_ref().expect("a local offer");

        let mut media: Vec<MediaDescription> = vec![];

        for (local, remote) in local_offer.media.iter().zip(remote_offer.media.iter()) {
            if local.media_type != remote.media_type {
                todo!("return err");
            }

            if local.proto != remote.proto {
                todo!("return err");
            }

            if remote.port == 0 || local.port == 0 {
                continue;
            }

            let local_dir = local
                .attributes
                .iter()
                .find(|attr| {
                    let name = attr.name.as_str();
                    matches!(name, "recvonly" | "sendrecv" | "sendonly" | "inactive")
                })
                .map(|attr| attr.name.as_str());

            let remote_dir = remote
                .attributes
                .iter()
                .find(|attr| {
                    let name = attr.name.as_str();
                    matches!(name, "recvonly" | "sendrecv" | "sendonly" | "inactive")
                })
                .map(|attr| attr.name.as_str());

            let answer_dir = match remote_dir {
                Some("sendonly") => match local_dir {
                    Some("sendrecv") | Some("recvonly") => Some("recvonly".to_owned()),
                    _ => Some("inactive".to_owned()),
                },
                Some("inactive") => Some("inactive".to_owned()),
                Some("recvonly") => match local_dir {
                    Some("sendrecv") | Some("sendonly") => Some("sendonly".to_owned()),
                    _ => Some("inactive".to_owned()),
                },
                Some("sendrecv") => Some("sendrecv".to_owned()),
                Some(_unknow) => todo!("return err"),
                None => None,
            };

            let attributes = if let Some(media_direction) = answer_dir {
                let mut media_attrs: Vec<_> = local
                    .attributes
                    .iter()
                    .filter(|attr| {
                        let name = attr.name.as_str();
                        !matches!(name, "recvonly" | "sendrecv" | "sendonly" | "inactive")
                    })
                    .map(ToOwned::to_owned)
                    .collect();

                media_attrs.push(Attribute {
                    name: media_direction,
                    value: None,
                });
                media_attrs
            } else {
                local.attributes.clone()
            };

            let mut media_formats = vec![];

            for media_format in &local.media_formats {
                let payload_type: u8 = media_format.parse::<u8>()?;

                if payload_type < 96 && remote.media_formats.contains(&media_format) {
                    media_formats.push(media_format.to_owned());
                } else {
                    // TODO: dynamic payload type
                    unimplemented!("dynamic payload type");
                }
            }

            media.push(MediaDescription {
                media_formats,
                attributes,
                ..local.clone()
            });
        }

        let answer = SessionDescription {
            media,
            time: remote_offer.time.clone(),
            ..local_offer.clone()
        };
        let answer_ref = &*self.answer.insert(answer);

        self.state = NegotiatorState::Done;

        Ok(answer_ref)
    }

    pub fn answer(&self) -> Option<&SessionDescription> {
        self.answer.as_ref()
    }

    pub fn add_local_media_stream(&mut self, stream: MediaStream) {
        unimplemented!()
    }
}

impl MediaStream {
    pub fn builder() -> MediaStreamBuilder {
        Default::default()
    }
}

impl MediaStreamBuilder {
    pub fn with_media_type(mut self, media_type: MediaType) -> Self {
        self.media_type = Some(media_type);
        self
    }

    pub fn with_codec(mut self, codec: Codec) -> Self {
        self.codecs.push(codec);
        self
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    pub fn with_transport(mut self, transport: SdpTransport) -> Self {
        self.transport = Some(transport);
        self
    }

    pub fn build(self) -> Result<MediaStream> {
        todo!()
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::sdp::parser::SdpParser;

    #[test]
    fn test_simple_offer_answer_exchange() {
        let offer = concat!(
            "v=0\r\n",
            "o=Tesla 2890844526 2890844526 IN IP4 lab.high-voltage.org\r\n",
            "s=-\r\n",
            "c=IN IP4 100.101.102.103\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0 8\r\n",
            "a=rtpmap:0 PCMU/8000\r\n",
            "a=rtpmap:8 PCMA/8000\r\n",
        );

        let answer = concat!(
            "v=0\r\n",
            "o=Marconi 2890844526 2890844526 IN IP4 tower.radio.org\r\n",
            "s=-\r\n",
            "c=IN IP4 200.201.202.203\r\n",
            "t=0 0\r\n",
            "m=audio 60000 RTP/AVP 8\r\n",
            "a=rtpmap:8 PCMA/8000\r\n",
        );

        let remote_offer = SdpParser::parse(offer).unwrap();
        let local_sdp = SdpParser::parse(answer).unwrap();

        let mut nego = SdpNegotiator::with_remote(remote_offer);

        // Subject
        // let sdp = SdpSessionConfig::new("sipecho", IpAddr::from_str("200.201.202.203").unwrap());

        nego.set_local_sdp(local_sdp).unwrap();

        let media_stream = MediaStream::builder()
            .with_media_type(MediaType::Audio)
            .with_transport(SdpTransport::RTPAVP)
            .with_codec(Codec::ULAW)
            .with_port(60000)
            .build()
            .unwrap();

        nego.add_local_media_stream(media_stream);

        let answer = nego.create_answer().unwrap();

        println!("{}", answer.encode_sdp().unwrap());
    }
}
