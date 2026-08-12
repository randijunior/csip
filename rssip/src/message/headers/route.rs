use std::fmt;

use crate::error::Result;
use crate::macros;
use crate::message::params::Params;
use crate::message::uri::NameAddr;
use crate::parser::{HeaderParse, SipParser};

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Route {
    pub(crate) name_addr: NameAddr,
    pub(crate) params: Params,
}

impl HeaderParse for Route {
    const NAME: &'static str = "Route";

    fn parse(parser: &mut SipParser) -> Result<Self> {
        let name_addr = parser.parse_name_addr()?;
        let params = macros::parse_params!(parser);
        Ok(Self { name_addr, params })
    }
}

impl fmt::Display for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", Self::NAME)?;

        write!(f, "{}", self.name_addr)?;

        write!(f, "{}", self.params)?;

        Ok(())
    }
}
