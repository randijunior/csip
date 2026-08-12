use std::fmt;

use crate::error::Result;
use crate::macros;
use crate::message::params::Params;
use crate::message::uri::SipUri;
use crate::parser::{HeaderParse, SipParser};

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ReplyTo {
    uri: SipUri,
    param: Params,
}

impl HeaderParse for ReplyTo {
    const NAME: &'static str = "Reply-To";

    fn parse(parser: &mut SipParser) -> Result<Self> {
        let uri = parser.parse_sip_uri(false)?;
        let param = macros::parse_params!(parser);

        Ok(Self { uri, param })
    }
}

impl fmt::Display for ReplyTo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", Self::NAME, self.uri)?;
        write!(f, "{}", self.param)?;
        Ok(())
    }
}
