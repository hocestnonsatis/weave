//! `weave status` implementation.

use std::path::Path;

use weave_core::{
    DependencyStatus, EnvironmentStatus, GitStatus, MaterializationStatus, ProjectStatus,
    WEAVE_BACKUP_NODE_MODULES, WEAVE_CANDIDATE_DIR, WEAVE_DIR,
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

    let leftover_candidate = layout
        .root
        .join(WEAVE_DIR)
        .join(WEAVE_CANDIDATE_DIR)
        .exists();
    let leftover_backup = layout
        .root
        .join(WEAVE_DIR)
        .join(WEAVE_BACKUP_NODE_MODULES)
        .exists();

    let active_matches_lockfile = match (&active_environment, &graph_identity) {
        (Some(active), Some(gid)) => known
            .iter()
            .find(|e| e.id.as_str() == active.as_str())
            .map(|e| e.graph_identity.as_str() == gid.as_str()),
        _ => None,
    };

    let next_steps = compute_next_steps(
        layout.weave_initialized,
        layout.lockfile.is_some(),
        parse_error.is_some(),
        active_environment.is_some(),
        node_modules_present,
        leftover_candidate || leftover_backup,
        active_matches_lockfile,
        repo.working_tree.dependency_files_dirty,
    );

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
            graph_identity: graph_identity.clone(),
            parse_error,
        },
        materialization: MaterializationStatus {
            node_modules_present,
            active_environment: active_environment.clone(),
        },
        environment: EnvironmentStatus {
            known_count,
            branch_association,
            environments: known
                .into_iter()
                .map(|e| {
                    let active = active_environment.as_deref() == Some(e.id.as_str());
                    let matches_lockfile = graph_identity
                        .as_ref()
                        .map(|g| g == e.graph_identity.as_str());
                    weave_core::EnvironmentSummary {
                        id: e.id.to_string(),
                        label: e.label,
                        owner: e.owner,
                        package_count: e.package_count,
                        active,
                        matches_lockfile,
                        created_at: e.created_at,
                        last_activated_at: e.last_activated_at,
                    }
                })
                .collect(),
        },
        project: layout,
        next_steps,
    })
}

#[allow(clippy::too_many_arguments)]
fn compute_next_steps(
    initialized: bool,
    lockfile_present: bool,
    parse_error: bool,
    has_active: bool,
    node_modules_present: bool,
    needs_recover: bool,
    active_matches_lockfile: Option<bool>,
    dependency_files_dirty: bool,
) -> Vec<String> {
    let mut steps = Vec::new();
    if !lockfile_present {
        steps.push(
            "create package-lock.json with `npm install` or `npm i --package-lock-only` \
             (Yarn/pnpm-only projects are unsupported)"
                .into(),
        );
        return steps;
    }
    if parse_error {
        steps.push("fix package-lock.json parse errors, then weave doctor --json".into());
        return steps;
    }
    if !initialized {
        steps.push("weave init --json".into());
        steps.push("weave doctor --json".into());
        steps.push("weave switch --json".into());
        return steps;
    }
    if needs_recover {
        steps.push("weave recover --json".into());
    }
    if dependency_files_dirty {
        steps.push(
            "dependency files dirty vs HEAD — commit or stash before relying on branch labels"
                .into(),
        );
    }
    if !has_active || !node_modules_present || active_matches_lockfile == Some(false) {
        steps.push("weave switch --json".into());
    }
    steps.push("weave status --json".into());
    if has_active && active_matches_lockfile != Some(false) && node_modules_present {
        steps.push("weave doctor --json".into());
    }
    steps
}
