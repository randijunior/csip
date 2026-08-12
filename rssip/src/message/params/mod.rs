pub mod q;

use std::{fmt, str};

pub use q::*;

use crate::{error, parser};
pub(crate) type ParamRef<'a> = (&'a str, Option<&'a str>);

pub(crate) const TAG_PARAM: &str = "tag";
pub(crate) const Q_PARAM: &str = "q";
pub(crate) const EXPIRES_PARAM: &str = "expires";
pub(crate) const USER_PARAM: &str = "user";
pub(crate) const METHOD_PARAM: &str = "method";
pub(crate) const TRANSPORT_PARAM: &str = "transport";
pub(crate) const TTL_PARAM: &str = "ttl";
pub(crate) const LR_PARAM: &str = "lr";
pub(crate) const MADDR_PARAM: &str = "maddr";
pub(crate) const BRANCH_PARAM: &str = "branch";
pub(crate) const RPORT_PARAM: &str = "rport";
pub(crate) const RECEIVED_PARAM: &str = "received";

#[derive(Default, Debug, PartialEq, Eq, Clone)]
pub struct Params {
    inner: Vec<Param>,
}

#[derive(Debug, PartialEq, Eq, Default, Clone)]
pub struct Param {
    /// The parameter name.
    pub name: String,
    /// The optional parameter value
    pub value: Option<String>,
}

impl Params {
    /// Get value of the first param field with a given name.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.inner
            .iter()
            .find(|Param { name: p_name, .. }| p_name == name)
            .map(|Param { value, .. }| value.as_deref())?
    }

    /// Returns an iterator over the parameters.
    pub fn iter(&self) -> impl Iterator<Item = &Param> {
        self.inner.iter()
    }

    /// Pushes a new parameter into collection.
    pub fn push(&mut self, param: Param) {
        self.inner.push(param)
    }

    /// Returns the number of elements.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` if the param list contains no elements.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl fmt::Display for Params {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for Param { name, value } in &self.inner {
            write!(f, ";{}", name)?;
            if let Some(v) = value {
                write!(f, "={}", v)?;
            }
        }
        Ok(())
    }
}

impl From<ParamRef<'_>> for Param {
    #[inline]
    fn from((name, value): ParamRef) -> Self {
        Self {
            name: name.to_owned(),
            value: value.map(ToOwned::to_owned),
        }
    }
}

impl str::FromStr for Param {
    type Err = error::Error;

    fn from_str(s: &str) -> error::Result<Self> {
        Ok(parser::SipParser::new(s).param_ref()?.into())
    }
}
