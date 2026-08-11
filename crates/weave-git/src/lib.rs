//! Git adapter for Weave.
//!
//! Version 1 shells out to the `git` CLI. The rest of Weave depends only on
//! the types and functions exported here, so a library-backed implementation
//! can replace this later without domain changes.

#![deny(missing_docs)]

mod cli;
mod status;

pub use cli::GitCli;
pub use status::{GitRepository, WorkingTreeState};
