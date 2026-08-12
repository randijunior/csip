use std::{fmt, str};

use crate::error::Result;
use crate::macros;
use crate::message::params::Params;
use crate::parser::{HeaderParse, SipParser};

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct AlertInfo {
    url: String,
    params: Params,
}

impl HeaderParse for AlertInfo {
    const NAME: &'static str = "Alert-Info";

    fn parse(parser: &mut SipParser) -> Result<Self> {
        parser.skip_ws();
        parser.must_read(b'<')?;
        let url = parser.take_until(b'>');
        parser.advance()?;

        let url = str::from_utf8(url)?.to_owned();

        let params = macros::parse_params!(parser);

        Ok(AlertInfo { url, params })
    }
}

impl fmt::Display for AlertInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: <{}>", Self::NAME, self.url)?;
        write!(f, "{}", self.params)?;
        Ok(())
    }
}
