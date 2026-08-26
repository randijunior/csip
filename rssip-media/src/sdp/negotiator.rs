use std::net::IpAddr;

use crate::codec::Codec;
use crate::error::{Error, Result};
use crate::sdp::{
    AddrType, Attribute, ConnectionInformation, Direction, MediaDescription, MediaType, NetType,
    Origin, SdpTransport, SessionDescription, TimeActive, TimeDescription,
};

#[derive(Default)]
pub struct Negotiator {
    remote_offer: Option<SessionDescription>,
    local_offer: Option<SessionDescription>,
    answer: Option<SessionDescription>,
    state: NegotiationState,
}

/// SDP Negotiation state.
#[derive(Default, Debug, PartialEq, Eq)]
enum NegotiationState {
    #[default]
    Initial,
    LocalOffer,
    RemoteOffer,
    Ready,
    Done,
}

pub struct SdpOfferParams {
    direction: Direction,
    origin_ip: IpAddr,
    media_streams: Vec<SdpMediaStream>,
}

pub struct SdpMediaStream {
    transport: SdpTransport,
    media_type: MediaType,
    codecs: Vec<Codec>,
    port: u16,
}

impl Negotiator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_local(local: SessionDescription) -> Self {
        Self {
            local_offer: Some(local),
            state: NegotiationState::LocalOffer,
            answer: None,
            remote_offer: None,
        }
    }

    pub fn with_remote(remote: SessionDescription) -> Self {
        Self {
            remote_offer: Some(remote),
            state: NegotiationState::RemoteOffer,
            answer: None,
            local_offer: None,
        }
    }

    pub fn set_remote_offer(&mut self, remote: SessionDescription) -> Result<()> {
        self.state = match self.state {
            NegotiationState::Initial => NegotiationState::RemoteOffer,
            NegotiationState::LocalOffer => NegotiationState::Ready,
            _ => return Err(Error::ErrInvalidNegoState),
        };
        self.remote_offer = Some(remote);
        Ok(())
    }

    pub fn set_local_offer(&mut self, local: SessionDescription) -> Result<()> {
        self.state = match self.state {
            NegotiationState::Initial => NegotiationState::LocalOffer,
            NegotiationState::RemoteOffer => NegotiationState::Ready,
            _ => return Err(Error::ErrInvalidNegoState),
        };
        self.local_offer = Some(local);
        Ok(())
    }

    // RFC 3264 5 - Generating the Initial Offer
    pub fn generate_offer(&mut self, params: SdpOfferParams) -> &SessionDescription {
        // the set of media streams and codecs the
        // offerer wishes to use, along with the IP addresses and ports the
        // offerer would like to use to receive the media.

        // an SDP message used in the offer/answer model MUST
        // contain exactly one session description.
        let offer_ref = &*self.local_offer.get_or_insert_with(|| {
            let mut media: Vec<MediaDescription> = params
                .media_streams
                .into_iter()
                .map(|media_stream| {
                    let mut media_formats = vec![];
                    let mut attributes = vec![];

                    for codec in media_stream.codecs.iter() {
                        match codec.name() {
                            "PCMU" => {
                                attributes.push(Attribute {
                                    name: "rtpmap".to_owned(),
                                    value: Some("0 PCMU/8000".to_owned()),
                                });
                            }
                            "PCMA" => {
                                attributes.push(Attribute {
                                    name: "rtpmap".to_owned(),
                                    value: Some("8 PCMA/8000".to_owned()),
                                });
                            }
                            "opus" => {
                                attributes.push(Attribute {
                                    name: "rtpmap".to_owned(),
                                    value: Some("96 opus/48000/2".to_owned()),
                                });
                            }
                            "telephone-event" => {
                                attributes.push(Attribute {
                                    name: "rtpmap".to_owned(),
                                    value: Some("101 telephone-event/8000".to_owned()),
                                });
                                attributes.push(Attribute {
                                    name: "fmtp".to_owned(),
                                    value: Some("101 0-16".to_owned()),
                                });
                            }
                            _ => {
                                attributes.push(Attribute {
                                    name: "rtpmap".to_owned(),
                                    value: Some(format!(
                                        "{}/{}/{}/{}/",
                                        codec.pt(),
                                        codec.name(),
                                        codec.clock_rate(),
                                        codec.channels()
                                    )),
                                });
                            }
                        }
                        media_formats.push(codec.pt().to_string());
                    }

                    MediaDescription {
                        media_type: media_stream.media_type,
                        proto: media_stream.transport,
                        port: media_stream.port,
                        number_of_ports: None,
                        media_formats,
                        title: None,
                        connection_information: None,
                        bandwidth_information: vec![],
                        attributes,
                    }
                })
                .collect();

            if let Some(m) = media.last_mut() {
                m.attributes.push(Attribute {
                    name: "ptime".to_owned(),
                    value: Some("20".to_owned()),
                });

                m.attributes.push(Attribute {
                    name: "maxptime".to_owned(),
                    value: Some("150".to_owned()),
                });

                m.attributes.push(Attribute {
                    name: params.direction.to_string(),
                    value: None,
                });
            }

            SessionDescription {
                origin: Origin {
                    user: "-".to_owned(),
                    session_id: rand::random::<u64>(),
                    session_version: rand::random::<u64>(),
                    nettype: NetType::IN,
                    addrtype: if params.origin_ip.is_ipv4() {
                        AddrType::IP4
                    } else {
                        AddrType::IP6
                    },
                    unicast_address: params.origin_ip,
                },
                session_name: "-".to_owned(),
                session_information: None,
                uri: None,
                email_address: None,
                phone_number: None,
                connection_information: Some(ConnectionInformation {
                    nettype: NetType::IN,
                    addrtype: if params.origin_ip.is_ipv4() {
                        AddrType::IP4
                    } else {
                        AddrType::IP6
                    },
                    conection_address: params.origin_ip,
                }),
                bandwidth_information: vec![],
                attributes: vec![],
                time: vec![TimeDescription {
                    time_active: TimeActive {
                        start_time: 0,
                        stop_time: 0,
                    },
                    repeat_times: vec![],
                }],
                media,
            }
        });
        if self.state != NegotiationState::LocalOffer {
            self.state = NegotiationState::LocalOffer;
        }
        offer_ref
    }

    // RFC 3264 6 - Generating the Answer
    pub fn generate_answer(&mut self) -> Result<&SessionDescription> {
        if self.state != NegotiationState::Ready {
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

        self.state = NegotiationState::Done;

        Ok(answer_ref)
    }

    // // RFC 3264 7 - Offerer Processing of the Answer
    pub fn process_answer(&mut self) -> Result<()> {
        todo!()
    }

    pub fn local_offer(&self) -> Option<&SessionDescription> {
        self.local_offer.as_ref()
    }

    pub fn answer(&self) -> Option<&SessionDescription> {
        self.answer.as_ref()
    }
}

impl SdpOfferParams {
    pub fn new(origin_ip: IpAddr, direction: Direction) -> Self {
        Self {
            direction,
            origin_ip,
            media_streams: vec![],
        }
    }

    pub fn add_media_stream(mut self, stream: SdpMediaStream) -> Self {
        self.media_streams.push(stream);
        self
    }
}

impl SdpMediaStream {
    pub fn audio(port: u16) -> Self {
        Self {
            transport: SdpTransport::RTPAVP,
            media_type: MediaType::Audio,
            codecs: vec![],
            port,
        }
    }
    pub fn video(port: u16) -> Self {
        Self {
            transport: SdpTransport::RTPAVP,
            media_type: MediaType::Video,
            codecs: vec![],
            port,
        }
    }

    pub fn with_codecs(mut self, codecs: Vec<Codec>) -> Self {
        self.codecs = codecs;
        self
    }
}

#[cfg(test)]
mod tests {
    use utils::encode::Encode;

    use super::*;
    use crate::codec::Codec;
    use crate::sdp::Direction;
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

        let mut nego = Negotiator::with_remote(remote_offer);

        nego.set_local_offer(local_sdp).unwrap();

        let _answer = nego.generate_answer().unwrap();
    }

    #[test]
    fn test_generate_offer() {
        let mut negotiator = Negotiator::new();

        let audio = SdpMediaStream::audio(34391).with_codecs(vec![
            Codec::ULAW,
            Codec::ALAW,
            Codec::OPUS,
            Codec::TELEPHONE_EVENT,
        ]);

        let offer = SdpOfferParams::new("192.168.178.54".parse().unwrap(), Direction::SendRecv)
            .add_media_stream(audio);

        let offer = negotiator.generate_offer(offer);

        println!("{}", offer.encode().unwrap());
    }
}
