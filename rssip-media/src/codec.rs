use std::borrow::Cow;

#[derive(Debug, Clone)]
pub struct Codec {
    name: Cow<'static, str>,
    clock_rate: u32,                          // clock rate in Hz
    payload_type: u8,                         // pt
    channels: u16,                            // Number of audio channels (0 for video codecs)
    sdp_fmtp_line: Option<Cow<'static, str>>, // Format-specific parameters as SDP fmtp line
}

impl Codec {
    /// G.711 μ-law (RFC 3551 §4.5.14)
    pub const ULAW: Self = Self::new("PCMU", 8000, 0, 1, None);
    pub const ALAW: Self = Self::new("PCMA", 8000, 8, 1, None);
    pub const OPUS: Self = Self::new("opus", 48_000, 96, 2, None);
    pub const TELEPHONE_EVENT: Self = Self::new("telephone-event", 8000, 101, 1, Some("0-16"));

    pub const fn new(
        name: &'static str,
        clock_rate: u32,
        payload_type: u8,
        channels: u16,
        fmtp: Option<&'static str>,
    ) -> Self {
        Self {
            name: Cow::Borrowed(name),
            clock_rate,
            payload_type,
            channels,
            sdp_fmtp_line: if let Some(fmtp) = fmtp {
                Some(Cow::Borrowed(fmtp))
            } else {
                None
            },
        }
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn clock_rate(&self) -> u32 {
        self.clock_rate
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn pt(&self) -> u8 {
        self.payload_type
    }
}
