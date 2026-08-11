//! Project discovery models.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Supported lockfile kinds for the current Weave version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LockfileKind {
    /// npm `package-lock.json` (lockfileVersion 1–3).
    NpmPackageLock,
}

impl LockfileKind {
    /// Human-readable name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NpmPackageLock => "npm package-lock.json",
        }
    }
}

/// Filesystem layout of a Node.js project that Weave can manage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectLayout {
    /// Absolute path to the Git repository root (also the project root for MVP).
    pub root: PathBuf,
    /// Absolute path to `package.json`.
    pub package_json: PathBuf,
    /// Absolute path to the supported lockfile, if present.
    pub lockfile: Option<PathBuf>,
    /// Kind of the detected lockfile.
    pub lockfile_kind: Option<LockfileKind>,
    /// Whether `.weave/` exists.
    pub weave_initialized: bool,
}

/// Result of discovering a Weave-capable project from a starting directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectDiscovery {
    /// Discovered layout.
    pub layout: ProjectLayout,
}
