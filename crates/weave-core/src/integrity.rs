//! npm-style Subresource Integrity (SRI) helpers.

use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};

use crate::{Error, Result};

/// Parsed integrity specifier from a lockfile (`sha512-…`, `sha256-…`, `sha1-…`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Integrity {
    /// Hash algorithm.
    pub algorithm: IntegrityAlgo,
    /// Expected digest bytes.
    pub digest: Vec<u8>,
    /// Original SRI string.
    pub raw: String,
}

/// Supported integrity algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IntegrityAlgo {
    /// SHA-1 (legacy npm).
    Sha1,
    /// SHA-256.
    Sha256,
    /// SHA-512 (current npm default).
    Sha512,
}

impl Integrity {
    /// Parse an SRI string such as `sha512-abc…==`.
    pub fn parse(raw: &str) -> Result<Self> {
        let (algo_str, b64) = raw.split_once('-').ok_or_else(|| Error::InvalidState {
            path: std::path::PathBuf::from(raw),
            reason: "integrity must look like algorithm-base64".into(),
        })?;

        let algorithm = match algo_str {
            "sha1" => IntegrityAlgo::Sha1,
            "sha256" => IntegrityAlgo::Sha256,
            "sha512" => IntegrityAlgo::Sha512,
            other => {
                return Err(Error::InvalidState {
                    path: std::path::PathBuf::from(raw),
                    reason: format!("unsupported integrity algorithm: {other}"),
                });
            }
        };

        let digest = base64_decode(b64).map_err(|reason| Error::InvalidState {
            path: std::path::PathBuf::from(raw),
            reason,
        })?;

        Ok(Self {
            algorithm,
            digest,
            raw: raw.to_owned(),
        })
    }

    /// Compute a `sha256-…` SRI for `bytes` (offline; never fetches).
    ///
    /// Intended for humans who already independently verified a local artifact
    /// and need reviewable policy data — Weave still does not invent trust.
    pub fn sha256_sri(bytes: &[u8]) -> Self {
        let digest = Sha256::digest(bytes).to_vec();
        let raw = format!("sha256-{}", base64_encode(&digest));
        Self {
            algorithm: IntegrityAlgo::Sha256,
            digest,
            raw,
        }
    }

    /// Verify `bytes` against this integrity specifier.
    pub fn verify(&self, bytes: &[u8], package: &str) -> Result<()> {
        let actual = match self.algorithm {
            IntegrityAlgo::Sha1 => {
                use sha1::Digest as Sha1Digest;
                let _ = Sha1::new(); // keep type linked
                <Sha1 as Sha1Digest>::digest(bytes).to_vec()
            }
            IntegrityAlgo::Sha256 => Sha256::digest(bytes).to_vec(),
            IntegrityAlgo::Sha512 => Sha512::digest(bytes).to_vec(),
        };

        if actual.as_slice() != self.digest.as_slice() {
            return Err(Error::IntegrityCheckFailed {
                package: package.to_owned(),
                reason: format!("expected {}, got different digest", self.raw),
            });
        }
        Ok(())
    }
}

/// Minimal base64 decoder (std alphabet, ignores whitespace).
fn base64_decode(input: &str) -> std::result::Result<Vec<u8>, String> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let filtered: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
        .collect();

    if filtered.len() % 4 == 1 {
        return Err("invalid base64 length".into());
    }

    let mut out = Vec::with_capacity(filtered.len() * 3 / 4);
    let mut i = 0;
    while i + 4 <= filtered.len() {
        let a = val(filtered[i]).ok_or_else(|| "invalid base64".to_string())?;
        let b = val(filtered[i + 1]).ok_or_else(|| "invalid base64".to_string())?;
        let c = val(filtered[i + 2]).ok_or_else(|| "invalid base64".to_string())?;
        let d = val(filtered[i + 3]).ok_or_else(|| "invalid base64".to_string())?;
        out.push((a << 2) | (b >> 4));
        out.push((b << 4) | (c >> 2));
        out.push((c << 6) | d);
        i += 4;
    }

    let rem = filtered.len() - i;
    if rem == 2 {
        let a = val(filtered[i]).ok_or_else(|| "invalid base64".to_string())?;
        let b = val(filtered[i + 1]).ok_or_else(|| "invalid base64".to_string())?;
        out.push((a << 2) | (b >> 4));
    } else if rem == 3 {
        let a = val(filtered[i]).ok_or_else(|| "invalid base64".to_string())?;
        let b = val(filtered[i + 1]).ok_or_else(|| "invalid base64".to_string())?;
        let c = val(filtered[i + 2]).ok_or_else(|| "invalid base64".to_string())?;
        out.push((a << 2) | (b >> 4));
        out.push((b << 4) | (c >> 2));
    } else if rem != 0 {
        return Err("invalid base64 remainder".into());
    }

    Ok(out)
}

fn base64_encode(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let a = chunk[0] as u32;
        let b = chunk.get(1).copied().unwrap_or(0) as u32;
        let c = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (a << 16) | (b << 8) | c;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(T[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(T[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_sha256_sri() {
        let bytes = b"weave";
        let integrity = Integrity::sha256_sri(bytes);
        assert!(integrity.raw.starts_with("sha256-"));
        integrity.verify(bytes, "demo").unwrap();
        let err = integrity.verify(b"nope", "demo").unwrap_err();
        assert!(matches!(err, Error::IntegrityCheckFailed { .. }));
    }
}
