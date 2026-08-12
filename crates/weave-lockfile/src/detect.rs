//! Lockfile detection for supported npm formats.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use weave_core::{Error, LockfileKind};

/// Metadata about a detected lockfile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockfileInfo {
    /// Absolute path to the lockfile.
    pub path: PathBuf,
    /// Kind of lockfile.
    pub kind: LockfileKind,
    /// npm `lockfileVersion` when known.
    pub lockfile_version: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct PackageLockHeader {
    #[serde(rename = "lockfileVersion")]
    lockfile_version: Option<u32>,
}

/// Detect a supported lockfile under `root`.
///
/// Returns `Ok(None)` when no lockfile is present. Returns an error when a
/// lockfile exists but is unsupported or unreadable.
pub fn detect_lockfile(root: &Path) -> weave_core::Result<Option<LockfileInfo>> {
    let path = root.join("package-lock.json");
    if !path.is_file() {
        // Fail clearly when another package manager owns the tree — do not
        // silently look like a "missing npm install" case.
        for (name, label) in [
            ("pnpm-lock.yaml", "pnpm"),
            ("yarn.lock", "Yarn"),
            ("bun.lockb", "Bun"),
            ("bun.lock", "Bun"),
        ] {
            let alt = root.join(name);
            if alt.is_file() {
                return Err(Error::UnsupportedLockfile {
                    path: alt,
                    reason: format!(
                        "{label} lockfile present without package-lock.json; \
                         Weave currently supports npm package-lock.json only"
                    ),
                });
            }
        }
        return Ok(None);
    }

    let bytes = fs::read(&path).map_err(|source| Error::Io {
        path: path.clone(),
        source,
    })?;

    let header: PackageLockHeader =
        serde_json::from_slice(&bytes).map_err(|err| Error::UnsupportedLockfile {
            path: path.clone(),
            reason: format!("invalid JSON: {err}"),
        })?;

    let version = header.lockfile_version.unwrap_or(1);
    // npm lockfileVersion 1, 2, and 3 are commonly encountered today.
    if !(1..=3).contains(&version) {
        return Err(Error::UnsupportedLockfile {
            path: path.clone(),
            reason: format!("lockfileVersion {version} is not supported yet"),
        });
    }

    Ok(Some(LockfileInfo {
        path,
        kind: LockfileKind::NpmPackageLock,
        lockfile_version: Some(version),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_missing_lockfile() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(detect_lockfile(tmp.path()).unwrap().is_none());
    }

    #[test]
    fn detects_v3_lockfile() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("package-lock.json"),
            r#"{"name":"demo","lockfileVersion":3,"packages":{}}"#,
        )
        .unwrap();
        let info = detect_lockfile(tmp.path()).unwrap().unwrap();
        assert_eq!(info.kind, LockfileKind::NpmPackageLock);
        assert_eq!(info.lockfile_version, Some(3));
    }

    #[test]
    fn rejects_unsupported_version() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("package-lock.json"),
            r#"{"name":"demo","lockfileVersion":99}"#,
        )
        .unwrap();
        let err = detect_lockfile(tmp.path()).unwrap_err();
        assert!(matches!(err, Error::UnsupportedLockfile { .. }));
    }

    #[test]
    fn rejects_pnpm_only_projects_clearly() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("pnpm-lock.yaml"),
            "lockfileVersion: '9.0'\n",
        )
        .unwrap();
        let err = detect_lockfile(tmp.path()).unwrap_err();
        match err {
            Error::UnsupportedLockfile { reason, .. } => {
                assert!(reason.to_lowercase().contains("pnpm"));
            }
            other => panic!("expected UnsupportedLockfile, got {other}"),
        }
    }
}
