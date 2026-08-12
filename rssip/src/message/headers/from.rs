use core::fmt;
use std::str::{self, FromStr};

use crate::error::Result;
use crate::macros;
use crate::message::params::{self, Params};
use crate::message::uri::SipUri;
use crate::parser::{HeaderParse, SipParser};

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct From {
    pub uri: SipUri,
    pub tag: Option<String>,
    pub params: Params,
}

impl HeaderParse for From {
    const NAME: &'static str = "From";
    const SHORT_NAME: &'static str = "f";

    fn parse(parser: &mut SipParser) -> Result<Self> {
        let uri = parser.parse_sip_uri(false)?;
        let mut tag = None;
        let params = macros::parse_params!(parser, {
            let (pname, pvalue) = parser.param_ref()?;
            if pname == params::TAG_PARAM {
                tag = pvalue.map(ToOwned::to_owned);
                None
            } else {
                Some((pname, pvalue).into())
            }
        });

        Ok(Self { tag, uri, params })
    }
}

impl FromStr for From {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(&mut SipParser::new(s))
    }
}

impl From {
    /// Returns the tag parameter.
    pub fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }
}

impl fmt::Display for From {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", Self::NAME)?;

        match &self.uri {
            SipUri::Uri(uri) => write!(f, "{}", uri)?,
            SipUri::NameAddr(name_addr) => write!(f, "{}", name_addr)?,
        }
        if let Some(tag) = &self.tag {
            write!(f, ";tag={}", tag)?;
        }
        write!(f, "{}", self.params)?;

        Ok(())
    }
}
