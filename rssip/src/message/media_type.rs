use std::fmt;

use crate::error::Result;
use crate::macros;
use crate::message::params::Params;
use crate::parser::SipParser;

/// The `media-type` that appears in `Accept` and
/// `Content-Type` SIP headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaType {
    pub mimetype: MimeType,
    pub params: Params,
}

/// This type reprents an MIME type that indicates an
/// content format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MimeType {
    pub mtype: String,
    pub subtype: String,
}

impl MediaType {
    /// Constructs a `MediaType` from a type and a subtype.
    pub fn new(mtype: String, subtype: String) -> Self {
        Self {
            mimetype: MimeType { mtype, subtype },
            params: Default::default(),
        }
    }

    pub fn parse(parser: &mut SipParser) -> Result<Self> {
        let mtype = parser.token()?;
        parser.advance()?;
        let subtype = parser.token()?;
        let param = macros::parse_params!(parser);

        Ok(Self::from_parts(mtype, subtype, param))
    }

    pub fn from_static(s: &'static str) -> Result<Self> {
        Self::parse(&mut SipParser::new(s.as_bytes()))
    }

    /// Constructs a `MediaType` with an optional
    /// parameters.
    pub fn from_parts(mtype: &str, subtype: &str, params: Params) -> Self {
        Self {
            mimetype: MimeType {
                mtype: mtype.into(),
                subtype: subtype.into(),
            },
            params,
        }
    }
}

impl fmt::Display for MediaType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let MediaType { mimetype, params } = self;
        write!(f, "{}/{}", mimetype.mtype, mimetype.subtype)?;
        write!(f, "{}", params)?;
        Ok(())
    }
}
