//! Safe recovery from interrupted / stale Weave filesystem state.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use weave_core::{Error, WEAVE_BACKUP_NODE_MODULES, WEAVE_CANDIDATE_DIR, WEAVE_DIR};

use crate::environment::EnvironmentStore;
use crate::project::discover_project;
use crate::registry::register_project;

/// Options for [`recover_project`].
#[derive(Debug, Clone, Default)]
pub struct RecoverOpts {
    /// When true, also delete `.weave/node_modules.bak` after confirming it is
    /// only a leftover backup (never touches live `node_modules`).
    pub purge_backup: bool,
}

/// Report from a recovery pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecoverReport {
    /// Project root.
    pub root: PathBuf,
    /// Whether a leftover candidate directory was removed.
    pub removed_candidate: bool,
    /// Whether a leftover backup directory was removed.
    pub removed_backup: bool,
    /// Whether a dangling active pointer was cleared.
    pub cleared_dangling_active: bool,
    /// Whether the project was (re-)registered for shared-store GC.
    pub registered_project: bool,
    /// Human-readable actions taken / skipped.
    pub actions: Vec<String>,
    /// Suggested next commands.
    pub next_steps: Vec<String>,
}

/// Recover from interrupted switch / stale metadata without enabling exec or network.
///
/// Safe by default:
/// - removes leftover `.weave/candidate` (incomplete materialize)
/// - clears active pointer when the referenced environment record is missing
/// - does **not** delete live `node_modules`
/// - does **not** delete backup unless `purge_backup`
/// - does **not** rewrite lockfiles or enable execution
pub fn recover_project(start: &Path, opts: &RecoverOpts) -> weave_core::Result<RecoverReport> {
    let discovery = discover_project(start)?;
    let root = discovery.layout.root;
    if !discovery.layout.weave_initialized {
        return Err(Error::NotInitialized { root });
    }

    let mut actions = Vec::new();
    let mut removed_candidate = false;
    let mut removed_backup = false;
    let mut cleared_dangling_active = false;

    let candidate = root.join(WEAVE_DIR).join(WEAVE_CANDIDATE_DIR);
    if candidate.exists() {
        fs::remove_dir_all(&candidate).map_err(|source| Error::Io {
            path: candidate.clone(),
            source,
        })?;
        removed_candidate = true;
        actions.push(format!(
            "removed leftover candidate {}",
            candidate.display()
        ));
    } else {
        actions.push("no leftover candidate".into());
    }

    let backup = root.join(WEAVE_DIR).join(WEAVE_BACKUP_NODE_MODULES);
    if backup.exists() {
        if opts.purge_backup {
            fs::remove_dir_all(&backup).map_err(|source| Error::Io {
                path: backup.clone(),
                source,
            })?;
            removed_backup = true;
            actions.push(format!("removed leftover backup {}", backup.display()));
        } else {
            actions.push(format!(
                "left backup in place at {} (pass --purge-backup to delete)",
                backup.display()
            ));
        }
    } else {
        actions.push("no leftover backup".into());
    }

    let store = EnvironmentStore::open(&root);
    if let Some(active) = store.active_id()? {
        if store.get(&active).is_err() {
            store.clear_active_if(&active)?;
            cleared_dangling_active = true;
            actions.push(format!(
                "cleared dangling active pointer for missing environment {active}"
            ));
        } else {
            actions.push(format!("active environment {active} is consistent"));
        }
    } else {
        actions.push("no active environment pointer".into());
    }

    let registered_project = register_project(&root).is_ok();
    if registered_project {
        actions.push("registered project for shared-store GC".into());
    }

    let next_steps = vec![
        "weave doctor --json".into(),
        "weave switch --json".into(),
        "weave status --json".into(),
    ];

    Ok(RecoverReport {
        root,
        removed_candidate,
        removed_backup,
        cleared_dangling_active,
        registered_project,
        actions,
        next_steps,
    })
}
