//! Well-known paths inside a Weave project.

/// Project-local Weave metadata directory.
pub const WEAVE_DIR: &str = ".weave";

/// Project Weave configuration file (relative to [`WEAVE_DIR`]).
pub const WEAVE_CONFIG: &str = "config.toml";

/// Directory for environment metadata (relative to [`WEAVE_DIR`]).
pub const WEAVE_ENVIRONMENTS_DIR: &str = "environments";

/// Directory for miscellaneous project metadata (relative to [`WEAVE_DIR`]).
pub const WEAVE_METADATA_DIR: &str = "metadata";

/// Candidate materialization root (relative to [`WEAVE_DIR`]).
pub const WEAVE_CANDIDATE_DIR: &str = "candidate";

/// Backup of the previous `node_modules` during activation (relative to [`WEAVE_DIR`]).
pub const WEAVE_BACKUP_NODE_MODULES: &str = "backup-node_modules";
