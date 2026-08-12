use std::{fmt, str};

use crate::error::Result;
use crate::macros;
use crate::message::params::Params;
use crate::parser::{HeaderParse, SipParser};

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RetryAfter {
    seconds: u32,
    param: Params,
    comment: Option<String>,
}

impl HeaderParse for RetryAfter {
    const NAME: &'static str = "Retry-After";

    fn parse(parser: &mut SipParser) -> Result<Self> {
        let seconds = parser.parse_u32()?;
        let mut comment = None;

        parser.skip_ws();
        if let Some(b'(') = parser.peek() {
            parser.advance()?;
            let b = parser.take_until(b')');
            parser.advance()?;
            comment = Some(str::from_utf8(b)?);
        }
        let param = macros::parse_params!(parser);

        Ok(Self {
            seconds,
            param,
            comment: comment.map(|c| c.into()),
        })
    }
}

impl fmt::Display for RetryAfter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", Self::NAME)?;

        write!(f, "{}", self.seconds)?;

        write!(f, "{}", self.param)?;

        if let Some(comment) = &self.comment {
            write!(f, "{}", comment)?;
        }

        Ok(())
    }
}
