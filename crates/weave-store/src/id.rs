//! Artifact identity (SHA-256 content hash).

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use weave_core::Error;

/// Content-addressed artifact identifier (lowercase hex SHA-256).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ArtifactId(String);

impl ArtifactId {
    /// Parse a hex SHA-256 digest.
    pub fn parse(value: impl AsRef<str>) -> weave_core::Result<Self> {
        let value = value.as_ref();
        if value.len() != 64 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(Error::InvalidState {
                path: std::path::PathBuf::from(value),
                reason: "artifact id must be 64 lowercase/uppercase hex characters".into(),
            });
        }
        Ok(Self(value.to_ascii_lowercase()))
    }

    /// Create from an already-validated lowercase hex digest.
    pub(crate) fn from_hex_unchecked(hex: String) -> Self {
        Self(hex)
    }

    /// Borrow the hex digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// First two hex characters (directory shard).
    pub fn shard(&self) -> &str {
        &self.0[..2]
    }

    /// Remaining hex characters (object file name).
    pub fn object_name(&self) -> &str {
        &self.0[2..]
    }
}

impl fmt::Display for ArtifactId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Compute the [`ArtifactId`] for `bytes`.
pub fn hash_bytes(bytes: &[u8]) -> ArtifactId {
    let digest = Sha256::digest(bytes);
    ArtifactId::from_hex_unchecked(hex_encode(&digest))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_stable() {
        let id = hash_bytes(b"hello weave");
        assert_eq!(id.as_str().len(), 64);
        assert_eq!(id, hash_bytes(b"hello weave"));
        assert_ne!(id, hash_bytes(b"hello weave!"));
    }

    #[test]
    fn rejects_bad_ids() {
        assert!(ArtifactId::parse("abcd").is_err());
        assert!(ArtifactId::parse("g".repeat(64)).is_err());
    }
}
