//! A business id: 32 random bytes, lowercase hex.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::ProtocolError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BusinessId([u8; 32]);

impl BusinessId {
    /// A fresh id from the operating system's random source.
    pub fn generate() -> BusinessId {
        BusinessId(rand::random())
    }

    pub fn from_bytes(bytes: [u8; 32]) -> BusinessId {
        BusinessId(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for BusinessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

/// Only the canonical form parses: 64 lowercase hex characters. One business
/// therefore has exactly one spelling in tags, database keys and filters.
impl FromStr for BusinessId {
    type Err = ProtocolError;

    fn from_str(s: &str) -> Result<BusinessId, ProtocolError> {
        if s.len() != 64 || !s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(ProtocolError::BadBusinessId);
        }
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(s, &mut bytes).map_err(|_| ProtocolError::BadBusinessId)?;
        Ok(BusinessId(bytes))
    }
}

impl Serialize for BusinessId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for BusinessId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}
