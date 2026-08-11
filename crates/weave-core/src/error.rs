//! Shared error type for Weave crates.

use std::path::PathBuf;

/// Result alias using [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// Domain and infrastructure errors that Weave surfaces to the CLI.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The current directory is not inside a Git repository.
    #[error("not a Git repository (or any of the parent directories): {path}")]
    NotAGitRepository {
        /// Path that was searched from.
        path: PathBuf,
    },

    /// `package.json` is required for Weave project initialization.
    #[error("package.json not found in project root: {root}")]
    MissingPackageJson {
        /// Project root that was inspected.
        root: PathBuf,
    },

    /// No supported lockfile was found.
    #[error(
        "no supported lockfile found in {root}\n\
         Weave currently requires npm package-lock.json.\n\
         Run `npm install` (or `npm i --package-lock-only`) to create one, then retry."
    )]
    MissingLockfile {
        /// Project root that was inspected.
        root: PathBuf,
    },

    /// The lockfile format is not supported yet.
    #[error("unsupported lockfile at {path}: {reason}")]
    UnsupportedLockfile {
        /// Path to the lockfile.
        path: PathBuf,
        /// Human-readable reason.
        reason: String,
    },

    /// The lockfile could not be parsed into a dependency graph.
    #[error("invalid lockfile at {path}: {reason}")]
    InvalidLockfile {
        /// Path to the lockfile.
        path: PathBuf,
        /// Human-readable reason.
        reason: String,
    },

    /// An artifact was not found in the content store.
    #[error("artifact not found: {id}")]
    ArtifactNotFound {
        /// Missing artifact id.
        id: String,
    },

    /// Stored artifact content does not match its id.
    #[error("artifact hash mismatch for {id}: {reason}")]
    ArtifactHashMismatch {
        /// Artifact id that failed verification.
        id: String,
        /// Human-readable reason.
        reason: String,
    },

    /// Stored artifact is corrupt or incomplete.
    #[error("corrupt artifact {id}: {reason}")]
    CorruptArtifact {
        /// Artifact id.
        id: String,
        /// Human-readable reason.
        reason: String,
    },

    /// Downloaded or provided bytes failed integrity verification.
    #[error("integrity check failed for {package}: {reason}")]
    IntegrityCheckFailed {
        /// Package name or URL label.
        package: String,
        /// Human-readable reason.
        reason: String,
    },

    /// Network / fetch failure while acquiring an artifact.
    #[error("failed to fetch {url}: {reason}")]
    FetchFailed {
        /// Requested URL.
        url: String,
        /// Human-readable reason.
        reason: String,
    },

    /// Materialization failed (layout, extraction, or safety checks).
    #[error("materialization failed at {path}: {reason}")]
    MaterializationFailed {
        /// Path related to the failure.
        path: PathBuf,
        /// Human-readable reason.
        reason: String,
    },

    /// Weave has not been initialized in this project.
    #[error("Weave is not initialized in {root}\nRun `weave init` first.")]
    NotInitialized {
        /// Project root.
        root: PathBuf,
    },

    /// Weave is already initialized.
    #[error("Weave is already initialized in {root}")]
    AlreadyInitialized {
        /// Project root.
        root: PathBuf,
    },

    /// A Git command failed.
    #[error("git command failed: {message}")]
    Git {
        /// Diagnostic message.
        message: String,
    },

    /// Filesystem I/O failure.
    #[error("filesystem error at {path}: {source}")]
    Io {
        /// Path involved in the failure.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// Configuration or state file is invalid.
    #[error("invalid Weave state at {path}: {reason}")]
    InvalidState {
        /// Path to the bad state.
        path: PathBuf,
        /// Human-readable reason.
        reason: String,
    },

    /// Feature is intentionally not implemented yet.
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}
