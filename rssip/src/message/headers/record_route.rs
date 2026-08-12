use std::fmt;

use crate::error::Result;
use crate::macros;
use crate::message::params::Params;
use crate::message::uri::NameAddr;
use crate::parser::{HeaderParse, SipParser};

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RecordRoute {
    addr: NameAddr,
    params: Params,
}

impl RecordRoute {
    pub fn name_addr(&self) -> &NameAddr {
        &self.addr
    }
    pub fn params(&self) -> &Params {
        &self.params
    }
}

impl HeaderParse for RecordRoute {
    const NAME: &'static str = "Record-Route";

    fn parse(parser: &mut SipParser) -> Result<Self> {
        let addr = parser.parse_name_addr()?;
        let params = macros::parse_params!(parser);

        Ok(Self { addr, params })
    }
}

impl fmt::Display for RecordRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", Self::NAME, self.addr)?;
        write!(f, "{}", self.params)?;

        Ok(())
    }
}
