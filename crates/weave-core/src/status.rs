//! Status report models for `weave status`.

use serde::{Deserialize, Serialize};

use crate::project::{LockfileKind, ProjectLayout};

/// Git-facing slice of project status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitStatus {
    /// Absolute repository root.
    pub root: String,
    /// Current branch name, or `None` when detached.
    pub branch: Option<String>,
    /// Short HEAD commit hash.
    pub head: String,
    /// Whether the working tree has uncommitted changes.
    pub dirty: bool,
    /// Whether `package.json` or the lockfile differ from HEAD.
    pub dependency_files_dirty: bool,
}

/// Dependency-facing slice of project status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyStatus {
    /// Whether `package.json` exists.
    pub package_json: bool,
    /// Whether a supported lockfile exists.
    pub lockfile_present: bool,
    /// Detected lockfile kind.
    pub lockfile_kind: Option<LockfileKind>,
    /// Path to the lockfile relative to the project root, if any.
    pub lockfile_path: Option<String>,
    /// Number of non-root packages in the parsed graph, if parse succeeded.
    pub package_count: Option<usize>,
    /// Deterministic graph identity hex, if parse succeeded.
    pub graph_identity: Option<String>,
    /// Lockfile parse error message when the lockfile exists but is invalid.
    pub parse_error: Option<String>,
}

/// Materialized environment slice of project status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializationStatus {
    /// Whether `node_modules` currently exists on disk.
    pub node_modules_present: bool,
    /// Weave-managed active environment id, if any.
    pub active_environment: Option<String>,
}

/// Environment manager slice of project status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentStatus {
    /// Number of known Weave environments.
    pub known_count: usize,
    /// Branch → environment association for the current branch, if any.
    pub branch_association: Option<String>,
}

/// Aggregated project status returned by `weave status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectStatus {
    /// Whether Weave metadata is present.
    pub initialized: bool,
    /// Project layout summary.
    pub project: ProjectLayout,
    /// Git state.
    pub git: GitStatus,
    /// Dependency state.
    pub dependency: DependencyStatus,
    /// Materialized environment state.
    pub materialization: MaterializationStatus,
    /// Environment manager state.
    pub environment: EnvironmentStatus,
}
