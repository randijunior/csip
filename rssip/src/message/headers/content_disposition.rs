use core::fmt;

use crate::error::Result;
use crate::macros;
use crate::message::params::Params;
use crate::parser::{HeaderParse, SipParser};

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ContentDisposition {
    r#type: String,
    params: Params,
}

impl HeaderParse for ContentDisposition {
    const NAME: &'static str = "Content-Disposition";

    fn parse(parser: &mut SipParser) -> Result<Self> {
        let r#type = parser.token()?.to_owned();
        let params = macros::parse_params!(parser);

        Ok(Self { r#type, params })
    }
}

impl fmt::Display for ContentDisposition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", Self::NAME, self.r#type)?;

        write!(f, ";{}", self.params)?;

        Ok(())
    }
}
