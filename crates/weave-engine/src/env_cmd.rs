//! Helpers for `weave env` commands.

use std::collections::BTreeMap;
use std::path::Path;

use weave_core::Error;
use weave_git::GitRepository;
use weave_lockfile::parse_lockfile;

use crate::environment::{create_environment, EnvironmentRecord, EnvironmentStore};
use crate::project::discover_project;

/// Create an environment from the current lockfile.
pub fn env_create(start: &Path) -> weave_core::Result<EnvironmentRecord> {
    let discovery = discover_project(start)?;
    if !discovery.layout.weave_initialized {
        return Err(Error::NotInitialized {
            root: discovery.layout.root,
        });
    }
    let lockfile = discovery
        .layout
        .lockfile
        .ok_or_else(|| Error::MissingLockfile {
            root: discovery.layout.root.clone(),
        })?;
    let graph = parse_lockfile(&lockfile)?;
    let repo = GitRepository::inspect(&discovery.layout.root)?;
    create_environment(
        &discovery.layout.root,
        &graph,
        &BTreeMap::new(),
        repo.branch,
    )
}

/// List known environments for the project.
pub fn env_list(start: &Path) -> weave_core::Result<Vec<EnvironmentRecord>> {
    let discovery = discover_project(start)?;
    if !discovery.layout.weave_initialized {
        return Err(Error::NotInitialized {
            root: discovery.layout.root,
        });
    }
    EnvironmentStore::open(discovery.layout.root).list()
}
