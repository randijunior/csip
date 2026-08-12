use core::fmt;

use crate::error::Result;
use crate::macros;
use crate::message::params::{EXPIRES_PARAM, Params, Q, Q_PARAM};
use crate::message::uri::SipUri;
use crate::parser::{HeaderParse, SipParser};

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Contact {
    pub uri: SipUri,
    pub q: Option<Q>,
    pub expires: Option<u32>,
    pub param: Params,
}

impl HeaderParse for Contact {
    const NAME: &'static str = "Contact";
    const SHORT_NAME: &'static str = "m";

    fn parse(parser: &mut SipParser) -> Result<Self> {
        let uri = parser.parse_sip_uri(false)?;
        let mut q = None;
        let mut expires = None;
        let param = macros::parse_params!(parser, {
            let (pname, pvalue) = parser.param_ref()?;
            if pname == Q_PARAM {
                q = pvalue.map(std::str::FromStr::from_str).transpose()?;
                None
            } else if pname == EXPIRES_PARAM {
                expires = pvalue
                    .map(std::str::FromStr::from_str)
                    .transpose()
                    .or_else(|_| parser.error(crate::error::ParseErrorKind::Param))?;
                None
            } else {
                Some((pname, pvalue).into())
            }
        });

        Ok(Contact {
            uri,
            q,
            expires,
            param,
        })
    }
}

impl std::str::FromStr for Contact {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(&mut SipParser::new(s.as_bytes()))
    }
}

impl fmt::Display for Contact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", Self::NAME)?;

        write!(f, "{}", self.uri)?;

        if let Some(q) = self.q {
            write!(f, "{}", q)?;
        }
        if let Some(expires) = self.expires {
            write!(f, "{}", expires)?;
        }
        write!(f, "{}", self.param)?;
        Ok(())
    }
}
