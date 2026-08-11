//! npm `package-lock.json` support for Weave.
//!
//! Parses supported lockfile versions into a deterministic [`DependencyGraph`].

#![deny(missing_docs)]

mod detect;
mod parse;

pub use detect::{detect_lockfile, LockfileInfo};
pub use parse::{parse_lockfile, parse_lockfile_bytes, parse_project_lockfile};
pub use weave_core::{DependencyGraph, GraphIdentity};
