//! SIP Auth types
use std::fmt;

use crate::message::params::Params;

pub const CNONCE: &str = "cnonce";
pub const QOP: &str = "qop";
pub const NC: &str = "nc";
pub const NEXTNONCE: &str = "nextnonce";
pub const RSPAUTH: &str = "rspauth";
pub const DIGEST: &str = "Digest";
pub const REALM: &str = "realm";
pub const USERNAME: &str = "username";
pub const NONCE: &str = "nonce";
pub const URI: &str = "uri";
pub const RESPONSE: &str = "response";
pub const ALGORITHM: &str = "algorithm";
pub const OPAQUE: &str = "opaque";
pub const DOMAIN: &str = "domain";
pub const STALE: &str = "stale";

#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub struct DigestChallenge {
    pub realm: Option<String>,
    pub domain: Option<String>,
    pub nonce: Option<String>,
    pub opaque: Option<String>,
    pub stale: Option<String>,
    pub algorithm: Option<String>,
    pub qop: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Challenge {
    Digest(DigestChallenge),
    Other { scheme: String, params: Params },
}

impl fmt::Display for Challenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Challenge::Digest(DigestChallenge {
                realm,
                domain,
                nonce,
                opaque,
                stale,
                algorithm,
                qop,
            }) => {
                write!(f, "Digest ")?;
                if let Some(realm) = realm {
                    write!(f, "realm={realm}, ")?;
                }
                if let Some(domain) = domain {
                    write!(f, "domain={domain}, ")?;
                }
                if let Some(nonce) = nonce {
                    write!(f, "nonce={nonce}, ")?;
                }
                if let Some(opaque) = opaque {
                    write!(f, "opaque={opaque}, ")?;
                }
                if let Some(stale) = stale {
                    write!(f, "stale={stale}, ")?;
                }
                if let Some(algorithm) = algorithm {
                    write!(f, "algorithm={algorithm}, ")?;
                }
                if let Some(qop) = qop {
                    write!(f, "qop={qop}")?;
                }

                Ok(())
            }
            Challenge::Other {
                scheme: _,
                params: _,
            } => todo!(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DigestCredential {
    pub realm: Option<String>,
    pub username: Option<String>,
    pub nonce: Option<String>,
    pub uri: Option<String>,
    pub response: Option<String>,
    pub algorithm: Option<String>,
    pub cnonce: Option<String>,
    pub opaque: Option<String>,
    pub qop: Option<String>,
    pub nc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credential {
    Digest(DigestCredential),
    Other { scheme: String, param: Params },
}

impl fmt::Display for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Credential::Digest(DigestCredential {
                realm,
                username,
                nonce,
                uri,
                response,
                algorithm,
                cnonce,
                opaque,
                qop,
                nc,
            }) => {
                write!(f, "Digest ")?;
                if let Some(username) = username {
                    write!(f, "username={username}")?;
                }
                if let Some(realm) = realm {
                    write!(f, ", realm={realm}")?;
                }
                if let Some(nonce) = nonce {
                    write!(f, ", nonce={nonce}")?;
                }
                if let Some(uri) = uri {
                    write!(f, ", uri={uri}")?;
                }
                if let Some(response) = response {
                    write!(f, ", response={response}")?;
                }
                if let Some(algorithm) = algorithm {
                    write!(f, ", algorithm={algorithm}")?;
                }
                if let Some(cnonce) = cnonce {
                    write!(f, ", cnonce={cnonce}")?;
                }
                if let Some(qop) = qop {
                    write!(f, ", qop={qop}")?;
                }
                if let Some(nc) = nc {
                    write!(f, ", nc={nc}")?;
                }
                if let Some(opaque) = opaque {
                    write!(f, ", opaque={opaque}")?;
                }

                Ok(())
            }
            Credential::Other {
                scheme: _,
                param: _,
            } => todo!(),
        }
    }
}
