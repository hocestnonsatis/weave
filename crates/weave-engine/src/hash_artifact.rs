//! Offline hashing of user-verified local artifacts (Phase 13).
//!
//! Produces reviewable SRI / prebuild policy drafts from a file the human has
//! already obtained and verified independently. Never downloads, never enables
//! execution, and never invents trust — hashing is measurement only.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use weave_core::Integrity;

use crate::config::{parse_https_host, PrebuildFetchSpec};
use crate::exec_discover::validate_output_candidate_path;

/// Request to hash a local artifact into a reviewable draft.
#[derive(Debug, Clone)]
pub struct HashArtifactRequest {
    /// Path to a regular local file (symlinks refused).
    pub path: PathBuf,
    /// Package name for the draft fetch entry.
    pub package: String,
    /// Relative sealed output path under the package root.
    pub output: String,
    /// Optional HTTPS URL to include in the draft (not fetched).
    pub url: Option<String>,
    /// Optional Node ABI constraint.
    pub node_abi: Option<String>,
    /// Optional OS constraint.
    pub os: Option<String>,
    /// Optional CPU constraint.
    pub cpu: Option<String>,
}

/// Result of hashing a verified local artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HashArtifactReport {
    /// Absolute path that was hashed.
    pub path: String,
    /// Byte length.
    pub size_bytes: u64,
    /// Computed `sha256-…` SRI.
    pub integrity: String,
    /// Reviewable fetch draft when URL was provided and valid.
    pub draft: Option<PrebuildFetchSpec>,
    /// Host extracted from URL when present.
    pub host: Option<String>,
    /// Rendered TOML fragment for human review.
    pub toml_fragment: String,
    /// Explicit reminder that trust is not established by hashing alone.
    pub note: String,
}

/// Hash a local file the user claims to have independently verified.
///
/// Safety:
/// - refuses symlinks (follow tricks)
/// - refuses directories
/// - never contacts the network
/// - never writes config or enables execution
pub fn hash_verified_artifact(req: &HashArtifactRequest) -> weave_core::Result<HashArtifactReport> {
    validate_output_candidate_path(&req.output).map_err(|reason| {
        weave_core::Error::InvalidState {
            path: PathBuf::from(&req.output),
            reason,
        }
    })?;
    if req.output.contains('*') {
        return Err(weave_core::Error::InvalidState {
            path: PathBuf::from(&req.output),
            reason: "output must be an exact relative file path (no globs)".into(),
        });
    }
    if req.package.trim().is_empty() {
        return Err(weave_core::Error::InvalidState {
            path: PathBuf::from("package"),
            reason: "package name is required".into(),
        });
    }

    let meta = fs::symlink_metadata(&req.path).map_err(|source| weave_core::Error::Io {
        path: req.path.clone(),
        source,
    })?;
    if meta.file_type().is_symlink() {
        return Err(weave_core::Error::InvalidState {
            path: req.path.clone(),
            reason: "refusing to hash symlink — pass a regular file you verified \
                     (prevents accidental path / TOCTOU footguns)"
                .into(),
        });
    }
    if !meta.is_file() {
        return Err(weave_core::Error::InvalidState {
            path: req.path.clone(),
            reason: "path is not a regular file".into(),
        });
    }

    let mut file = fs::File::open(&req.path).map_err(|source| weave_core::Error::Io {
        path: req.path.clone(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|source| weave_core::Error::Io {
            path: req.path.clone(),
            source,
        })?;

    let integrity = Integrity::sha256_sri(&bytes);
    let (draft, host) = match &req.url {
        Some(url) => {
            let (scheme, host) =
                parse_https_host(url).map_err(|reason| weave_core::Error::InvalidState {
                    path: PathBuf::from(url),
                    reason,
                })?;
            if scheme != "https" {
                return Err(weave_core::Error::InvalidState {
                    path: PathBuf::from(url),
                    reason: "draft URL must be https (HTTP rejected)".into(),
                });
            }
            let spec = PrebuildFetchSpec {
                package: req.package.clone(),
                url: url.clone(),
                integrity: integrity.raw.clone(),
                output: req.output.clone(),
                node_abi: req.node_abi.clone(),
                os: req.os.clone(),
                cpu: req.cpu.clone(),
            };
            (Some(spec), Some(host))
        }
        None => (None, None),
    };

    let toml_fragment = render_hash_toml(&HashTomlInput {
        package: &req.package,
        integrity: &integrity.raw,
        output: &req.output,
        url: req.url.as_deref(),
        host: host.as_deref(),
        node_abi: req.node_abi.as_deref(),
        os: req.os.as_deref(),
        cpu: req.cpu.as_deref(),
    });

    Ok(HashArtifactReport {
        path: display_path(&req.path),
        size_bytes: bytes.len() as u64,
        integrity: integrity.raw,
        draft,
        host,
        toml_fragment,
        note: "REVIEW ONLY — hashing measures bytes; it does not prove provenance. \
               Confirm the file came from a trusted source before merging into \
               execution.prebuild.fetches. Never auto-enables execution or network."
            .into(),
    })
}

fn display_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

struct HashTomlInput<'a> {
    package: &'a str,
    integrity: &'a str,
    output: &'a str,
    url: Option<&'a str>,
    host: Option<&'a str>,
    node_abi: Option<&'a str>,
    os: Option<&'a str>,
    cpu: Option<&'a str>,
}

fn render_hash_toml(input: &HashTomlInput<'_>) -> String {
    let mut out = String::from(
        "# Generated by `weave exec hash-artifact` — REVIEW BEFORE USE.\n\
         # Human verification of the file is required; Weave did not invent trust.\n\
         # Does not set execution.enabled or profile=open.\n",
    );
    if let Some(host) = input.host {
        out.push_str("[execution.prebuild]\n");
        out.push_str(&format!("allow_hosts = [{host:?}]\n\n"));
    }
    if let Some(url) = input.url {
        out.push_str("[[execution.prebuild.fetches]]\n");
        out.push_str(&format!("package = {:?}\n", input.package));
        out.push_str(&format!("url = {url:?}\n"));
        out.push_str(&format!("integrity = {:?}\n", input.integrity));
        out.push_str(&format!("output = {:?}\n", input.output));
        if let Some(abi) = input.node_abi {
            out.push_str(&format!("node_abi = {abi:?}\n"));
        }
        if let Some(os) = input.os {
            out.push_str(&format!("os = {os:?}\n"));
        }
        if let Some(cpu) = input.cpu {
            out.push_str(&format!("cpu = {cpu:?}\n"));
        }
    } else {
        out.push_str(&format!(
            "# integrity = {:?}\n\
             # package = {:?}\n\
             # output = {:?}\n\
             # Add a concrete HTTPS url, then place under [[execution.prebuild.fetches]].\n",
            input.integrity, input.package, input.output
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_regular_file_and_refuses_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("addon.node");
        fs::write(&file, b"verified-bytes").unwrap();
        let report = hash_verified_artifact(&HashArtifactRequest {
            path: file.clone(),
            package: "demo".into(),
            output: "prebuilds/addon.node".into(),
            url: Some("https://cdn.example.com/addon.node".into()),
            node_abi: Some("137".into()),
            os: Some("linux".into()),
            cpu: Some("x64".into()),
        })
        .unwrap();
        assert!(report.integrity.starts_with("sha256-"));
        assert_eq!(report.size_bytes, b"verified-bytes".len() as u64);
        assert!(report.draft.is_some());
        assert!(!report.toml_fragment.contains("enabled = true"));
        Integrity::parse(&report.integrity)
            .unwrap()
            .verify(b"verified-bytes", "demo")
            .unwrap();

        let link = tmp.path().join("link.node");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&file, &link).unwrap();
            let err = hash_verified_artifact(&HashArtifactRequest {
                path: link,
                package: "demo".into(),
                output: "prebuilds/addon.node".into(),
                url: None,
                node_abi: None,
                os: None,
                cpu: None,
            })
            .unwrap_err();
            assert!(err.to_string().contains("symlink"));
        }
    }

    #[test]
    fn rejects_http_url_in_draft() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("x.node");
        fs::write(&file, b"x").unwrap();
        let err = hash_verified_artifact(&HashArtifactRequest {
            path: file,
            package: "demo".into(),
            output: "x.node".into(),
            url: Some("http://cdn.example.com/x.node".into()),
            node_abi: None,
            os: None,
            cpu: None,
        })
        .unwrap_err();
        assert!(err.to_string().to_ascii_lowercase().contains("https"));
    }
}
