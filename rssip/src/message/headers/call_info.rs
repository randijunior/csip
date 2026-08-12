use std::{fmt, str};

use crate::error::Result;
use crate::macros::{self};
use crate::message::params::Params;
use crate::parser::{HeaderParse, SipParser};

const PURPOSE: &str = "purpose";

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CallInfo {
    url: String,
    purpose: Option<String>,
    params: Params,
}

impl HeaderParse for CallInfo {
    const NAME: &'static str = "Call-Info";

    fn parse(parser: &mut SipParser) -> Result<Self> {
        let mut purpose: Option<String> = None;
        parser.must_read(b'<')?;
        let url = parser.take_until(b'>');
        parser.advance()?;
        let url = str::from_utf8(url)?.to_owned();
        let params = macros::parse_params!(parser, {
            let (pname, pvalue) = parser.param_ref()?;
            if pname == PURPOSE {
                purpose = pvalue.map(ToOwned::to_owned);
                None
            } else {
                Some((pname, pvalue).into())
            }
        });

        Ok(Self {
            url,
            params,
            purpose,
        })
    }
}

impl fmt::Display for CallInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: <{}>", Self::NAME, self.url)?;
        if let Some(purpose) = &self.purpose {
            write!(f, ";{}", purpose)?;
        }
        write!(f, "{}", self.params)?;
        Ok(())
    }
}
