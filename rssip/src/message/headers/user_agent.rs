use std::{fmt, str};

use crate::error::Result;
use crate::parser::{HeaderParse, SipParser};

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct UserAgent(pub String);

impl HeaderParse for UserAgent {
    const NAME: &'static str = "User-Agent";

    fn parse(parser: &mut SipParser) -> Result<Self> {
        let agent = parser.read_line()?;

        Ok(Self(agent.to_owned()))
    }
}

impl fmt::Display for UserAgent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", Self::NAME, self.0)
    }
}
