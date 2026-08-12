use std::{fmt, str};

use itertools::Itertools;

use crate::error::Result;
use crate::macros;
use crate::message::params::Params;
use crate::parser::{HeaderParse, SipParser};

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ErrorInfoUri {
    url: String,
    params: Params,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ErrorInfo(Vec<ErrorInfoUri>);

impl HeaderParse for ErrorInfo {
    const NAME: &'static str = "Error-Info";

    fn parse(parser: &mut SipParser) -> Result<Self> {
        let infos =
            macros::collect_elems_separated_by_comma!(parser, { ErrorInfoUri::parse(parser)? });

        Ok(Self(infos))
    }
}

impl ErrorInfoUri {
    pub fn parse(parser: &mut SipParser) -> Result<Self> {
        parser.must_read(b'<')?;
        let url = parser.take_until(b'>');
        parser.advance()?;

        let url = str::from_utf8(url)?.to_owned();
        let params = macros::parse_params!(parser);

        Ok(Self { url, params })
    }
}

impl fmt::Display for ErrorInfoUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.url)?;
        write!(f, ";{}", self.params)?;

        Ok(())
    }
}

impl fmt::Display for ErrorInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.iter().format(", "))
    }
}
