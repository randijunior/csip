use std::fmt;
use std::str::{
    FromStr, {self},
};

use crate::error::Result;
use crate::macros;
use crate::message::params::{Params, TAG_PARAM};
use crate::message::uri::SipUri;
use crate::parser::{HeaderParse, SipParser};

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct To {
    pub uri: SipUri,
    pub tag: Option<String>,
    pub params: Params,
}

impl To {
    /// Returns the tag parameter.
    pub fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }

    /// Set the tag parameter.
    pub fn set_tag(&mut self, tag: Option<String>) {
        self.tag = tag;
    }
}

impl HeaderParse for To {
    const NAME: &'static str = "To";
    const SHORT_NAME: &'static str = "t";

    fn parse(parser: &mut SipParser) -> Result<Self> {
        let uri = parser.parse_sip_uri(false)?;
        let mut tag: Option<String> = None;
        let params = macros::parse_params!(parser, {
            let (pname, pvalue) = parser.param_ref()?;
            if pname == TAG_PARAM {
                tag = pvalue.map(ToOwned::to_owned);
                None
            } else {
                Some((pname, pvalue).into())
            }
        });

        Ok(Self { tag, uri, params })
    }
}

impl FromStr for To {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(&mut SipParser::new(s.as_bytes()))
    }
}

impl fmt::Display for To {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", Self::NAME, self.uri)?;
        if let Some(tag) = &self.tag {
            write!(f, ";tag={}", tag)?;
        }
        write!(f, "{}", self.params)?;

        Ok(())
    }
}
