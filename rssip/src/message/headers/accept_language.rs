use std::str::FromStr;
use std::{fmt, str};

use itertools::Itertools;

use crate::error::Result;
use crate::macros;
use crate::message::params::{self, Params, Q};
use crate::parser::{HeaderParse, SipParser};

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct AcceptLanguage(Vec<Language>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Language {
    language: LanguageTag,
    q_param: Option<Q>,
    params: Params,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageTag(String);

impl HeaderParse for AcceptLanguage {
    const NAME: &'static str = "Accept-Language";

    fn parse(parser: &mut SipParser) -> Result<Self> {
        let languages = macros::collect_elems_separated_by_comma!(parser, {
            let language = LanguageTag::parse(parser);
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

            Language {
                language,
                q_param,
                params,
            }
        });

        Ok(Self(languages))
    }
}

impl fmt::Display for AcceptLanguage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", Self::NAME, self.0.iter().format(", "))
    }
}

impl Language {
    pub fn new(language: LanguageTag) -> Self {
        Self {
            language,
            q_param: None,
            params: Params::default(),
        }
    }

    pub fn from_parts(language: LanguageTag, q_param: Option<Q>, params: Params) -> Self {
        Self {
            language,
            q_param,
            params,
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Language {
            language,
            q_param,
            params,
        } = self;
        write!(f, "{}", language)?;
        if let Some(q) = q_param {
            write!(f, "{}", q)?;
        }
        write!(f, "{}", params)?;
        Ok(())
    }
}

impl LanguageTag {
    pub fn parse(parser: &mut SipParser) -> Self {
        let is_lang = |byte: u8| byte.is_ascii_alphabetic() || matches!(byte, b'*' | b'-');
        let tag = unsafe { parser.read_while_as_str_unchecked(is_lang) };

        Self(tag.to_owned())
    }
}

impl fmt::Display for LanguageTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)?;

        Ok(())
    }
}
