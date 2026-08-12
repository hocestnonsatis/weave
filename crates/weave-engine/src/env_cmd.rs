//! Helpers for `weave env` commands.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use weave_core::Error;
use weave_git::GitRepository;
use weave_lockfile::parse_lockfile;

use crate::environment::{
    create_environment_with_opts, CreateEnvironmentOpts, EnvironmentId, EnvironmentRecord,
    EnvironmentStore,
};
use crate::project::discover_project;

/// Options for [`env_create`].
#[derive(Debug, Clone, Default)]
pub struct EnvCreateOpts {
    /// Override label (default: current git branch when available).
    pub label: Option<String>,
    /// Explicit owner/session id (never auto-detected).
    pub owner: Option<String>,
}

/// Create an environment from the current lockfile.
pub fn env_create(start: &Path) -> weave_core::Result<EnvironmentRecord> {
    env_create_with_opts(start, &EnvCreateOpts::default())
}

/// Create an environment with explicit label/owner options.
pub fn env_create_with_opts(
    start: &Path,
    opts: &EnvCreateOpts,
) -> weave_core::Result<EnvironmentRecord> {
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
    let label = opts.label.clone().or(repo.branch);
    create_environment_with_opts(
        &discovery.layout.root,
        &graph,
        &BTreeMap::new(),
        CreateEnvironmentOpts {
            label,
            owner: opts.owner.clone(),
        },
    )
}

/// List known environments for the project.
pub fn env_list(start: &Path) -> weave_core::Result<Vec<EnvironmentRecord>> {
    env_list_filtered(start, None)
}

/// List environments, optionally filtered by exact owner string.
pub fn env_list_filtered(
    start: &Path,
    owner: Option<&str>,
) -> weave_core::Result<Vec<EnvironmentRecord>> {
    let discovery = discover_project(start)?;
    if !discovery.layout.weave_initialized {
        return Err(Error::NotInitialized {
            root: discovery.layout.root,
        });
    }
    let mut envs = EnvironmentStore::open(discovery.layout.root).list()?;
    if let Some(owner) = owner {
        envs.retain(|e| e.owner.as_deref() == Some(owner));
    }
    Ok(envs)
}

/// Result of removing one environment record.
#[derive(Debug, Clone, Serialize)]
pub struct EnvRemoveReport {
    /// Removed environment id.
    pub removed_id: String,
    /// Owner that was recorded, if any.
    pub owner: Option<String>,
    /// Label that was recorded, if any.
    pub label: Option<String>,
}

/// Remove a non-active environment by id / prefix / label.
///
/// Never mutates `node_modules`, never touches other projects, never enables
/// execution or network.
pub fn env_remove(start: &Path, target: &str) -> weave_core::Result<EnvRemoveReport> {
    let discovery = discover_project(start)?;
    if !discovery.layout.weave_initialized {
        return Err(Error::NotInitialized {
            root: discovery.layout.root,
        });
    }
    let store = EnvironmentStore::open(&discovery.layout.root);
    let record = store.resolve(target)?;
    let removed = store.remove(&record.id)?;
    Ok(EnvRemoveReport {
        removed_id: removed.id.to_string(),
        owner: removed.owner,
        label: removed.label,
    })
}

/// Options for [`env_prune`].
#[derive(Debug, Clone, Default)]
pub struct EnvPruneOpts {
    /// Required: only records with this exact owner are eligible.
    pub owner: String,
    /// When set, only prune records whose `last_activated_at` or `created_at`
    /// (unix-seconds string) is older than now − this many seconds. Records
    /// without timestamps are treated as eligible when owner matches.
    pub older_than_secs: Option<u64>,
    /// When true, report matches without deleting.
    pub dry_run: bool,
}

/// Result of pruning abandoned agent-owned environment records.
#[derive(Debug, Clone, Serialize)]
pub struct EnvPruneReport {
    /// Owner filter applied.
    pub owner: String,
    /// Whether this was a dry run.
    pub dry_run: bool,
    /// Environment ids removed (or that would be removed).
    pub removed_ids: Vec<String>,
    /// Active environment id skipped (if owned by the same owner).
    pub skipped_active: Option<String>,
    /// Count of owner-matching records that were too recent to prune.
    pub skipped_too_recent: usize,
}

/// Prune non-active environment records owned by an explicit agent/session id.
///
/// Deliberately requires `--owner`: Weave never auto-detects agents and will
/// not mass-delete unlabeled human environments.
pub fn env_prune(start: &Path, opts: &EnvPruneOpts) -> weave_core::Result<EnvPruneReport> {
    if opts.owner.trim().is_empty() {
        return Err(Error::InvalidState {
            path: start.to_path_buf(),
            reason: "env prune requires a non-empty --owner (never inferred)".into(),
        });
    }
    let discovery = discover_project(start)?;
    if !discovery.layout.weave_initialized {
        return Err(Error::NotInitialized {
            root: discovery.layout.root,
        });
    }
    let store = EnvironmentStore::open(&discovery.layout.root);
    let active = store.active_id()?;
    let now = now_unix_secs();
    let mut removed_ids = Vec::new();
    let mut skipped_active = None;
    let mut skipped_too_recent = 0usize;

    for env in store.list()? {
        if env.owner.as_deref() != Some(opts.owner.as_str()) {
            continue;
        }
        if active.as_ref() == Some(&env.id) {
            skipped_active = Some(env.id.to_string());
            continue;
        }
        if let Some(max_age) = opts.older_than_secs {
            let stamp = env
                .last_activated_at
                .as_deref()
                .or(env.created_at.as_deref())
                .and_then(|s| s.parse::<u64>().ok());
            if let Some(ts) = stamp {
                if now.saturating_sub(ts) < max_age {
                    skipped_too_recent += 1;
                    continue;
                }
            }
        }
        if opts.dry_run {
            removed_ids.push(env.id.to_string());
        } else {
            let removed = store.remove(&env.id)?;
            removed_ids.push(removed.id.to_string());
        }
    }
    removed_ids.sort();
    Ok(EnvPruneReport {
        owner: opts.owner.clone(),
        dry_run: opts.dry_run,
        removed_ids,
        skipped_active,
        skipped_too_recent,
    })
}

/// Look up one environment (id / prefix / label) with active flag.
pub fn env_show(start: &Path, target: &str) -> weave_core::Result<EnvListEntry> {
    let discovery = discover_project(start)?;
    if !discovery.layout.weave_initialized {
        return Err(Error::NotInitialized {
            root: discovery.layout.root,
        });
    }
    let store = EnvironmentStore::open(&discovery.layout.root);
    let active = store.active_id()?;
    let graph_id = discovery
        .layout
        .lockfile
        .as_ref()
        .and_then(|p| parse_lockfile(p).ok())
        .map(|g| g.identity().to_string());
    let record = store.resolve(target)?;
    Ok(entry_from_record(
        &record,
        active.as_ref(),
        graph_id.as_deref(),
    ))
}

/// Machine-readable list entry used by status / env list --json.
#[derive(Debug, Clone, Serialize)]
pub struct EnvListEntry {
    /// Environment id.
    pub id: String,
    /// Optional label.
    pub label: Option<String>,
    /// Optional owner/session.
    pub owner: Option<String>,
    /// Package count.
    pub package_count: usize,
    /// Graph identity.
    pub graph_identity: String,
    /// Whether this is the active environment pointer.
    pub active: bool,
    /// Whether graph identity matches the current lockfile (when known).
    pub matches_lockfile: Option<bool>,
    /// Creation stamp.
    pub created_at: Option<String>,
    /// Last activation stamp.
    pub last_activated_at: Option<String>,
}

/// Build list entries for JSON status / env list.
pub fn env_list_entries(
    start: &Path,
    owner: Option<&str>,
) -> weave_core::Result<Vec<EnvListEntry>> {
    let discovery = discover_project(start)?;
    if !discovery.layout.weave_initialized {
        return Err(Error::NotInitialized {
            root: discovery.layout.root,
        });
    }
    let store = EnvironmentStore::open(&discovery.layout.root);
    let active = store.active_id()?;
    let graph_id = discovery
        .layout
        .lockfile
        .as_ref()
        .and_then(|p| parse_lockfile(p).ok())
        .map(|g| g.identity().to_string());
    let mut out = Vec::new();
    for env in store.list()? {
        if let Some(owner) = owner {
            if env.owner.as_deref() != Some(owner) {
                continue;
            }
        }
        out.push(entry_from_record(
            &env,
            active.as_ref(),
            graph_id.as_deref(),
        ));
    }
    Ok(out)
}

fn entry_from_record(
    env: &EnvironmentRecord,
    active: Option<&EnvironmentId>,
    graph_id: Option<&str>,
) -> EnvListEntry {
    let matches_lockfile = graph_id.map(|g| g == env.graph_identity.as_str());
    EnvListEntry {
        id: env.id.to_string(),
        label: env.label.clone(),
        owner: env.owner.clone(),
        package_count: env.package_count,
        graph_identity: env.graph_identity.to_string(),
        active: active == Some(&env.id),
        matches_lockfile,
        created_at: env.created_at.clone(),
        last_activated_at: env.last_activated_at.clone(),
    }
}

fn now_unix_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
