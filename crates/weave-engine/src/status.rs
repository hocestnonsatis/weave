//! `weave status` implementation.

use std::path::Path;

use weave_core::{
    DependencyStatus, EnvironmentStatus, GitStatus, MaterializationStatus, ProjectStatus,
};
use weave_git::GitRepository;
use weave_lockfile::parse_lockfile;

use crate::environment::EnvironmentStore;
use crate::project::discover_project;

/// Collect a human- and machine-readable project status snapshot.
pub fn project_status(start: &Path) -> weave_core::Result<ProjectStatus> {
    let discovery = discover_project(start)?;
    let layout = discovery.layout;
    let repo = GitRepository::inspect(&layout.root)?;

    let node_modules_present = layout.root.join("node_modules").exists();
    let env_store = EnvironmentStore::open(&layout.root);
    let known = if layout.weave_initialized {
        env_store.list()?
    } else {
        Vec::new()
    };
    let known_count = known.len();
    let active_environment = if layout.weave_initialized {
        env_store.active_id()?.map(|id| id.to_string())
    } else {
        None
    };
    let branch_association = repo.branch.as_ref().and_then(|branch| {
        known
            .iter()
            .find(|e| e.label.as_deref() == Some(branch.as_str()))
            .map(|e| e.id.to_string())
    });

    let lockfile_path = layout.lockfile.as_ref().map(|path| {
        path.strip_prefix(&layout.root)
            .unwrap_or(path)
            .display()
            .to_string()
    });

    let (package_count, graph_identity, parse_error) = match &layout.lockfile {
        Some(path) => match parse_lockfile(path) {
            Ok(graph) => (
                Some(graph.package_count()),
                Some(graph.identity().to_string()),
                None,
            ),
            Err(err) => (None, None, Some(err.to_string())),
        },
        None => (None, None, None),
    };

    Ok(ProjectStatus {
        initialized: layout.weave_initialized,
        git: GitStatus {
            root: layout.root.display().to_string(),
            branch: repo.branch.clone(),
            head: repo.head,
            dirty: repo.working_tree.dirty,
            dependency_files_dirty: repo.working_tree.dependency_files_dirty,
        },
        dependency: DependencyStatus {
            package_json: true,
            lockfile_present: layout.lockfile.is_some(),
            lockfile_kind: layout.lockfile_kind,
            lockfile_path,
            package_count,
            graph_identity,
            parse_error,
        },
        materialization: MaterializationStatus {
            node_modules_present,
            active_environment,
        },
        environment: EnvironmentStatus {
            known_count,
            branch_association,
        },
        project: layout,
    })
}
