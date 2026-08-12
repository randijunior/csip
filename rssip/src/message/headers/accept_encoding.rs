use std::str::FromStr;
use std::{fmt, str};

use itertools::Itertools;

use crate::error::Result;
use crate::macros;
use crate::message::params::{self, Params, Q};
use crate::parser::{HeaderParse, SipParser};

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct AcceptEncoding(Vec<Coding>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coding {
    coding: String,
    q_param: Option<Q>,
    params: Params,
}

impl HeaderParse for AcceptEncoding {
    const NAME: &'static str = "Accept-Encoding";

    fn parse(parser: &mut SipParser) -> Result<Self> {
        let codings = macros::collect_elems_separated_by_comma!(parser, {
            let coding = parser.token()?.to_owned();
            let mut q_param = None;

            let params = macros::parse_params!(parser, {
                let (pname, pvalue) = parser.param_ref()?;

                if pname == params::Q_PARAM {
                    q_param = pvalue.map(Q::from_str).transpose()?;

                    None
                } else {
                    Some((pname, pvalue).into())
                }
            });

            Coding {
                coding,
                q_param,
                params,
            }
        });

        Ok(Self(codings))
    }
}

impl fmt::Display for AcceptEncoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", Self::NAME, self.0.iter().format(", "))
    }
}

impl fmt::Display for Coding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Coding {
            coding,
            q_param,
            params,
        } = self;

        write!(f, "{}", coding)?;
        if let Some(q) = q_param {
            write!(f, ";q={}.{}", q.0, q.1)?;
        }
        write!(f, "{}", params)?;
        Ok(())
    }
}
