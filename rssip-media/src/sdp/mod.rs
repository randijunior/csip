use std::fmt::{self, Formatter, Result as FmtResult, Write};
use std::net::IpAddr;

use crate::error::Error;

pub mod parser;

pub type Uri = String;

pub type SessionName = String;

pub type SessionInformation = String;

pub type EmailAddress = String;

pub type PhoneNumber = String;

#[derive(Default, Clone, Debug)]
pub struct SessionDescription {
    // v=  (proto version)
    // o=  (originator and session identifier)
    pub origin: Origin,
    // s=  (session name)
    pub session_name: SessionName,
    // i=* (session information)
    pub session_information: Option<SessionInformation>,
    // u=* (URI of description)
    pub uri: Option<Uri>,
    // e=* (email address)
    pub email_address: Option<EmailAddress>,
    // p=* (phone number)
    pub phone_number: Option<PhoneNumber>,
    // c=* (connection information)
    pub connection_information: Option<ConnectionInformation>,
    // b=* (zero or more bandwidth information lines)
    pub bandwidth_information: Vec<BandwidthInformation>,
    // a=* (zero or more session attribute lines)
    pub attributes: Vec<Attribute>,
    // Time description
    pub time: Vec<TimeDescription>,
    //  Media description, if present
    pub media: Vec<MediaDescription>,
}

impl fmt::Display for SessionDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { 
        // v=0
        write!(f, "v=0\r\n")?;

        // o=<username> <sess-id> <sess-version> <nettype> <addrtype> <unicast-address>
        write!(
            f,
            "o={} {} {} {} {} {}\r\n",
            self.origin.user,
            self.origin.session_id,
            self.origin.session_version,
            self.origin.nettype,
            self.origin.addrtype,
            self.origin.unicast_address
        )?;

        // s=<session name>
        write!(f, "s={}\r\n", self.session_name)?;

        // i=<session information>
        if let Some(session_information) = &self.session_information {
            write!(f, "i={}\r\n", session_information)?;
        }
        // u=<uri>
        if let Some(uri) = &self.uri {
            write!(f, "u={}\r\n", uri)?;
        }
        // e=<email-address>
        if let Some(email_address) = &self.email_address {
            write!(f, "e={}\r\n", email_address)?;
        }
        // p=<phone-number>
        if let Some(phone_number) = &self.phone_number {
            write!(f, "p={}\r\n", phone_number)?;
        }
        // c=<nettype> <addrtype> <connection-address>
        if let Some(c) = &self.connection_information {
            write!(
                f,
                "c={} {} {}\r\n",
                c.nettype, c.addrtype, c.conection_address
            )?;
        }
        //  b=<bwtype>:<bandwidth>
        for b in self.bandwidth_information.iter() {
            write!(f, "b={}:{}\r\n", b.bwtype, b.bandwidth)?;
        }
        // t=<start-time> <stop-time>
        for t in self.time.iter() {
            write!(
                f,
                "t={} {}\r\n",
                t.time_active.start_time, t.time_active.stop_time
            )?;
            // r=<repeat interval> <active duration> <offsets from start-time>
            for r in t.repeat_times.iter() {
                write!(f, "r={} {}", r.repeat_interval, r.active_duration)?;
                for offset in r.offsets.iter() {
                    write!(f, " {}", offset)?;
                }
                write!(f, "\r\n")?;
            }
        }

        for attr in self.attributes.iter() {
            match attr {
                Attribute {
                    name,
                    value: Some(v),
                } => {
                    write!(f, "a={}:{}\r\n", name, v)?;
                }
                Attribute { name, value: None } => {
                    write!(f, "a={}\r\n", name)?;
                }
            }
        }

        // m=<media> <port> <proto> <fmt> ...
        for m in self.media.iter() {
            write!(f, "m={} {}", m.media_type, m.port)?;
            if let Some(n) = m.number_of_ports {
                write!(f, "/{}", n)?;
            }
            write!(f, " {}", m.proto)?;

            for fmt in m.media_formats.iter() {
                write!(f, " {}", fmt)?;
            }
            write!(f, "\r\n")?;

            if let Some(title) = &m.title {
                write!(f, "t={}\r\n", title)?;
            }

            // c=* (connection information -- optional if included at session level)
            if let Some(c) = &m.connection_information {
                write!(
                    f,
                    "c={} {} {}\r\n",
                    c.nettype, c.addrtype, c.conection_address
                )?;
            }

            // b=* (zero or more bandwidth information lines)
            for b in m.bandwidth_information.iter() {
                write!(f, "b={}:{}\r\n", b.bwtype, b.bandwidth)?;
            }

            // a=* (zero or more media attribute lines)
            for attr in m.attributes.iter() {
                match attr {
                    Attribute {
                        name,
                        value: Some(v),
                    } => {
                        write!(f, "a={}:{}\r\n", name, v)?;
                    }
                    Attribute { name, value: None } => {
                        write!(f, "a={}\r\n", name)?;
                    }
                }
            }
        }

        Ok(())
    }
}
impl utils::encode::Encode for SessionDescription {
    type Buffer = String;
    type Error = std::fmt::Error;

    fn encode(&self) -> Result<Self::Buffer, Self::Error> {
        let mut buff = String::new();
        write!(buff,"{self}")?;
        Ok(buff)
    }
}

#[derive(Clone, Debug)]
pub struct TimeDescription {
    // t=  (time the session is active)
    pub time_active: TimeActive,

    // r=* (zero or more repeat times)
    // r=<repeat interval> <active duration> <offsets from start-time>
    pub repeat_times: Vec<RepeatTimes>, // z=* (optional time zone offset line)
}

#[derive(Clone, Debug)]
pub struct RepeatTimes {
    pub repeat_interval: i64,
    pub active_duration: i64,
    pub offsets: Vec<i64>,
}

#[derive(Default, Clone, Debug)]
pub enum AddrType {
    #[default]
    IP4,
    IP6,
}

impl fmt::Display for AddrType {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        use self::AddrType::*;

        f.write_str(match self {
            IP4 => "IP4",
            IP6 => "IP6",
        })
    }
}

#[derive(Default, Clone, Debug)]
pub enum NetType {
    #[default]
    IN,
    Other(String),
}

impl fmt::Display for NetType {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        use self::NetType::*;

        f.write_str(match self {
            IN => "IN",
            Other(str) => str.as_str(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct ConnectionInformation {
    pub nettype: NetType,
    pub addrtype: AddrType,
    pub conection_address: IpAddr,
}

#[derive(Clone, Debug)]
pub struct TimeActive {
    pub start_time: u64,
    pub stop_time: u64,
}

#[derive(Clone, Debug)]
pub enum Bwtype {
    CT,
    AS,
    RR,
    RS,
    TIAS,
    Other(String),
}

impl fmt::Display for Bwtype {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        use self::Bwtype::*;

        f.write_str(match self {
            CT => "CT",
            AS => "AS",
            RR => "RR",
            RS => "RS",
            TIAS => "TIAS",
            Other(str) => str.as_str(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct BandwidthInformation {
    pub bwtype: Bwtype,
    pub bandwidth: u64,
}

#[derive(Debug, Clone)]
pub struct Attribute {
    pub name: String,
    pub value: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Origin {
    pub user: String,
    pub session_id: u64,
    pub session_version: u64,
    pub nettype: NetType,
    pub addrtype: AddrType,
    pub unicast_address: IpAddr,
}

impl Default for Origin {
    fn default() -> Self {
        Self {
            user: Default::default(),
            session_id: Default::default(),
            session_version: Default::default(),
            nettype: Default::default(),
            addrtype: Default::default(),
            unicast_address: utils::local_ip::get_local_ip_addr(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Copy)]
pub enum MediaType {
    #[default]
    Audio,
    Video,
    Text,
    Application,
    Message,
}

impl fmt::Display for MediaType {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        use self::MediaType::*;

        f.write_str(match self {
            Audio => "audio",
            Video => "video",
            Text => "text",
            Application => "application",
            Message => "message",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum SdpTransport {
    UDP,
    RTPAVP,
    RTPAVPF,
    RTPSAVP,
    RTPSAVPF,
}

impl fmt::Display for SdpTransport {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        use self::SdpTransport::*;

        f.write_str(match self {
            UDP => "udp",
            RTPAVP => "RTP/AVP",
            RTPAVPF => "RTP/AVPF",
            RTPSAVP => "RTP/SAVP",
            RTPSAVPF => "RTP/SAVPF",
        })
    }
}

#[derive(Clone, Debug)]
pub struct MediaDescription {
    // m=  (media name and transport address)
    // m=<media> <port>/<number of ports> <proto> <fmt>
    pub media_type: MediaType,
    pub proto: SdpTransport,
    pub port: u16,
    pub number_of_ports: Option<usize>,
    pub media_formats: Vec<String>,
    // i=* (media title)
    pub title: Option<String>,
    // c=* (connection information -- optional if included at session level)
    pub connection_information: Option<ConnectionInformation>,
    // b=* (zero or more bandwidth information lines)
    pub bandwidth_information: Vec<BandwidthInformation>,
    // a=* (zero or more media attribute lines)
    pub attributes: Vec<Attribute>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RtpMap {
    pub payload_type: String,
    pub enc_name: String,
    pub clock_rate: u32,
    pub param: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Default)]
pub enum Direction {
    Unspecified,
    #[default]
    SendRecv,
    RecvOnly,
    SendOnly,
    Inactive,
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Direction::SendRecv => "sendrecv",
            Direction::SendOnly => "sendonly",
            Direction::RecvOnly => "recvonly",
            Direction::Inactive => "inactive",
            _ => "Unspecified",
        };
        write!(f, "{s}")
    }
}

impl SessionDescription {
    pub fn set_origin(&mut self, origin: Origin) {
        self.origin = origin;
    }

    pub fn set_name(&mut self, session_name: SessionName) {
        self.session_name = session_name;
    }

    pub fn set_information(&mut self, info: SessionInformation) {
        if let Some(media) = self.last_media_desc_mut() {
            media.title = Some(info);
        } else {
            self.session_information = Some(info);
        }
    }

    pub fn set_email_addr(&mut self, email: EmailAddress) {
        self.email_address = Some(email);
    }

    pub fn set_uri(&mut self, uri: Uri) {
        self.uri = Some(uri);
    }

    pub fn set_attr(&mut self, attr: Attribute) {
        if let Some(media) = self.last_media_desc_mut() {
            media.attributes.push(attr);
        } else {
            self.attributes.push(attr);
        }
    }

    pub fn set_bandwidth_information(&mut self, bandwidth: BandwidthInformation) {
        self.bandwidth_information.push(bandwidth);
    }

    pub fn set_media_description(&mut self, media: MediaDescription) {
        self.media.push(media);
    }

    pub fn last_media_desc_mut(&mut self) -> Option<&mut MediaDescription> {
        self.media.last_mut()
    }

    fn last_time_desc_mut(&mut self) -> Option<&mut TimeDescription> {
        self.time.last_mut()
    }

    pub fn set_time_desc(&mut self, time: TimeDescription) {
        self.time.push(time);
    }
    pub fn set_repeat_times(&mut self, time: RepeatTimes) -> Result<(), Error> {
        if let Some(timing) = self.last_time_desc_mut() {
            timing.repeat_times.push(time);
            Ok(())
        } else {
            Err(Error::SdpTimeDescriptionNotFound)
        }
    }
    pub fn set_phone(&mut self, phone: PhoneNumber) {
        self.phone_number = Some(phone);
    }
    pub fn set_connection(&mut self, conn: ConnectionInformation) {
        if let Some(media) = self.last_media_desc_mut() {
            media.connection_information = Some(conn);
        } else {
            self.connection_information = Some(conn);
        }
    }
}
