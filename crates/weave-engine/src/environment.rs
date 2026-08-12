//! Environment identity and metadata (Milestone 6).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use weave_core::{
    DependencyGraph, Error, GraphIdentity, HostPlatform, WEAVE_DIR, WEAVE_ENVIRONMENTS_DIR,
};
use weave_fs::materialization_version;

/// Stable environment identity string.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EnvironmentId(String);

impl EnvironmentId {
    /// Borrow the id string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Derive an environment id from graph + platform + materialization format.
    pub fn derive(graph: &DependencyGraph, platform: &PlatformIdentity) -> Self {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(b"weave-env-v1\0");
        hasher.update(graph.identity().as_str().as_bytes());
        hasher.update(b"\0");
        hasher.update(platform.os.as_bytes());
        hasher.update(b"\0");
        hasher.update(platform.arch.as_bytes());
        hasher.update(b"\0");
        hasher.update(materialization_version().as_bytes());
        let digest = hasher.finalize();
        Self(hex(&digest))
    }
}

impl std::fmt::Display for EnvironmentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Platform slice that participates in environment identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformIdentity {
    /// OS family (`linux`, `macos`, `windows`, …).
    pub os: String,
    /// CPU architecture (`x86_64`, `aarch64`, …).
    pub arch: String,
}

impl PlatformIdentity {
    /// Capture the current host platform.
    pub fn host() -> Self {
        let h = HostPlatform::current();
        Self {
            os: h.os,
            arch: h.arch,
        }
    }

    /// Convert to weave-core host platform for filtering.
    pub fn to_host(&self) -> HostPlatform {
        HostPlatform {
            os: self.os.clone(),
            arch: self.arch.clone(),
        }
    }
}

/// Persisted environment metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentRecord {
    /// Environment id.
    pub id: EnvironmentId,
    /// Graph identity that produced this environment.
    pub graph_identity: GraphIdentity,
    /// Platform identity.
    pub platform: PlatformIdentity,
    /// Materialization format version.
    pub materialization_version: String,
    /// npm lockfileVersion.
    pub lockfile_version: u32,
    /// Package count (non-root).
    pub package_count: usize,
    /// Optional human label (e.g. branch name association is separate).
    pub label: Option<String>,
    /// Caller-supplied owner/session id (never auto-detected; agents must pass explicitly).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// RFC3339 creation time (set on first persist of this id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// RFC3339 last successful activation time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activated_at: Option<String>,
    /// Map of package key → artifact id hex for materialization.
    pub artifacts: BTreeMap<String, String>,
}

/// Options when creating / refreshing an environment record.
#[derive(Debug, Clone, Default)]
pub struct CreateEnvironmentOpts {
    /// Optional label (defaults to git branch when callers supply it).
    pub label: Option<String>,
    /// Explicit owner/session. Never inferred from process/AI presence.
    pub owner: Option<String>,
}

/// Manage `.weave/environments/` records.
#[derive(Debug, Clone)]
pub struct EnvironmentStore {
    root: PathBuf,
}

impl EnvironmentStore {
    /// Open the environment store for a project root.
    pub fn open(project_root: impl Into<PathBuf>) -> Self {
        Self {
            root: project_root.into(),
        }
    }

    fn dir(&self) -> PathBuf {
        self.root.join(WEAVE_DIR).join(WEAVE_ENVIRONMENTS_DIR)
    }

    fn path_for(&self, id: &EnvironmentId) -> PathBuf {
        self.dir().join(format!("{}.json", id.as_str()))
    }

    fn active_path(&self) -> PathBuf {
        self.root
            .join(WEAVE_DIR)
            .join(weave_core::WEAVE_METADATA_DIR)
            .join("active")
    }

    /// Create or replace an environment record.
    pub fn save(&self, record: &EnvironmentRecord) -> weave_core::Result<()> {
        let dir = self.dir();
        fs::create_dir_all(&dir).map_err(|source| Error::Io {
            path: dir.clone(),
            source,
        })?;
        let path = self.path_for(&record.id);
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(record).map_err(|err| Error::InvalidState {
            path: path.clone(),
            reason: err.to_string(),
        })?;
        fs::write(&tmp, body).map_err(|source| Error::Io {
            path: tmp.clone(),
            source,
        })?;
        fs::rename(&tmp, &path).map_err(|source| Error::Io { path, source })?;
        Ok(())
    }

    /// Load one environment by id.
    pub fn get(&self, id: &EnvironmentId) -> weave_core::Result<EnvironmentRecord> {
        let path = self.path_for(id);
        let bytes = fs::read(&path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                Error::InvalidState {
                    path: path.clone(),
                    reason: format!("environment {id} not found"),
                }
            } else {
                Error::Io {
                    path: path.clone(),
                    source,
                }
            }
        })?;
        serde_json::from_slice(&bytes).map_err(|err| Error::InvalidState {
            path,
            reason: err.to_string(),
        })
    }

    /// List all known environments (sorted by id).
    pub fn list(&self) -> weave_core::Result<Vec<EnvironmentRecord>> {
        let dir = self.dir();
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|source| Error::Io {
            path: dir.clone(),
            source,
        })? {
            let entry = entry.map_err(|source| Error::Io {
                path: dir.clone(),
                source,
            })?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let bytes = fs::read(&path).map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
            let record: EnvironmentRecord =
                serde_json::from_slice(&bytes).map_err(|err| Error::InvalidState {
                    path,
                    reason: err.to_string(),
                })?;
            out.push(record);
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    /// Read the active environment id pointer, if any.
    pub fn active_id(&self) -> weave_core::Result<Option<EnvironmentId>> {
        let path = self.active_path();
        match fs::read_to_string(&path) {
            Ok(s) => {
                let id = s.trim();
                if id.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(EnvironmentId(id.to_owned())))
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(Error::Io { path, source }),
        }
    }

    /// Atomically set the active environment pointer (metadata only).
    pub fn set_active(&self, id: &EnvironmentId) -> weave_core::Result<()> {
        // Ensure the environment exists first.
        let _ = self.get(id)?;
        let path = self.active_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, format!("{}\n", id.as_str())).map_err(|source| Error::Io {
            path: tmp.clone(),
            source,
        })?;
        fs::rename(&tmp, &path).map_err(|source| Error::Io { path, source })
    }

    /// Clear the active pointer when it currently names `id` (metadata only).
    pub fn clear_active_if(&self, id: &EnvironmentId) -> weave_core::Result<()> {
        match self.active_id()? {
            Some(active) if &active == id => {
                let path = self.active_path();
                match fs::remove_file(&path) {
                    Ok(()) => Ok(()),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(source) => Err(Error::Io { path, source }),
                }
            }
            _ => Ok(()),
        }
    }

    /// Remove an environment record. Refuses if it is the active environment.
    pub fn remove(&self, id: &EnvironmentId) -> weave_core::Result<EnvironmentRecord> {
        let record = self.get(id)?;
        if self.active_id()?.as_ref() == Some(id) {
            return Err(Error::InvalidState {
                path: self.path_for(id),
                reason: format!(
                    "refusing to remove active environment {id}; switch away or deactivate first"
                ),
            });
        }
        let path = self.path_for(id);
        fs::remove_file(&path).map_err(|source| Error::Io { path, source })?;
        Ok(record)
    }

    /// Resolve an id, id-prefix, or label to a unique environment.
    pub fn resolve(&self, target: &str) -> weave_core::Result<EnvironmentRecord> {
        let envs = self.list()?;
        let matches: Vec<_> = envs
            .into_iter()
            .filter(|e| {
                e.label.as_deref() == Some(target)
                    || e.id.as_str() == target
                    || e.id.as_str().starts_with(target)
            })
            .collect();
        match matches.as_slice() {
            [one] => Ok(one.clone()),
            [] => Err(Error::InvalidState {
                path: self.dir(),
                reason: format!("no environment matches '{target}'"),
            }),
            many => Err(Error::InvalidState {
                path: self.dir(),
                reason: format!(
                    "ambiguous environment target '{target}' matches {} records; use a longer id",
                    many.len()
                ),
            }),
        }
    }

    /// Stamp `last_activated_at` and persist.
    pub fn mark_activated(&self, id: &EnvironmentId) -> weave_core::Result<EnvironmentRecord> {
        let mut record = self.get(id)?;
        record.last_activated_at = Some(utc_now_rfc3339());
        self.save(&record)?;
        Ok(record)
    }
}

/// Create an environment record from the project's lockfile graph and artifacts.
pub fn create_environment(
    project_root: &Path,
    graph: &DependencyGraph,
    artifacts: &BTreeMap<weave_core::PackageKey, weave_store::ArtifactId>,
    label: Option<String>,
) -> weave_core::Result<EnvironmentRecord> {
    create_environment_with_opts(
        project_root,
        graph,
        artifacts,
        CreateEnvironmentOpts { label, owner: None },
    )
}

/// Create / refresh an environment with explicit ownership options.
pub fn create_environment_with_opts(
    project_root: &Path,
    graph: &DependencyGraph,
    artifacts: &BTreeMap<weave_core::PackageKey, weave_store::ArtifactId>,
    opts: CreateEnvironmentOpts,
) -> weave_core::Result<EnvironmentRecord> {
    let platform = PlatformIdentity::host();
    let id = EnvironmentId::derive(graph, &platform);
    let store = EnvironmentStore::open(project_root);
    let prior = store.get(&id).ok();
    let mut artifact_map = BTreeMap::new();
    for (key, artifact) in artifacts {
        artifact_map.insert(key.as_str().to_owned(), artifact.to_string());
    }
    let owner = opts
        .owner
        .or_else(|| prior.as_ref().and_then(|p| p.owner.clone()));
    let created_at = prior
        .as_ref()
        .and_then(|p| p.created_at.clone())
        .unwrap_or_else(utc_now_rfc3339);
    let last_activated_at = prior.as_ref().and_then(|p| p.last_activated_at.clone());
    let label = opts
        .label
        .or_else(|| prior.as_ref().and_then(|p| p.label.clone()));
    let record = EnvironmentRecord {
        id,
        graph_identity: graph.identity(),
        platform,
        materialization_version: materialization_version().to_owned(),
        lockfile_version: graph.lockfile_version,
        package_count: graph.package_count(),
        label,
        owner,
        created_at: Some(created_at),
        last_activated_at,
        artifacts: artifact_map,
    };
    store.save(&record)?;
    let _ = crate::registry::register_project(project_root);
    Ok(record)
}

fn utc_now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Compact UTC stamp without pulling a time crate dependency.
    format!("{secs}")
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use weave_core::{LockfileKind, PackageKey, PackageNode, PackageSource};

    #[test]
    fn save_list_get_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        // Minimal weave layout
        fs::create_dir_all(tmp.path().join(WEAVE_DIR).join(WEAVE_ENVIRONMENTS_DIR)).unwrap();
        let mut nodes = BTreeMap::new();
        nodes.insert(
            PackageKey::root(),
            PackageNode {
                key: PackageKey::root(),
                name: Some("demo".into()),
                version: Some("1.0.0".into()),
                source: PackageSource::Workspace,
                integrity: None,
                dependencies: BTreeMap::new(),
                dev_dependencies: BTreeMap::new(),
                optional_dependencies: BTreeMap::new(),
                peer_dependencies: BTreeMap::new(),
                peer_dependencies_meta: BTreeMap::new(),
                has_install_script: false,
                optional: false,
                dev: false,
                peer: false,
                cpu: Vec::new(),
                os: Vec::new(),
                bundled_dependencies: Vec::new(),
                is_workspace: true,
                is_link: false,
                likely_native: false,
                bin: BTreeMap::new(),
            },
        );
        let graph = DependencyGraph {
            lockfile_kind: LockfileKind::NpmPackageLock,
            lockfile_version: 3,
            root: PackageKey::root(),
            nodes,
            edges: Vec::new(),
        };
        let record =
            create_environment(tmp.path(), &graph, &BTreeMap::new(), Some("main".into())).unwrap();
        let store = EnvironmentStore::open(tmp.path());
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, record.id);
        assert_eq!(
            store.get(&record.id).unwrap().label.as_deref(),
            Some("main")
        );
    }

    #[test]
    fn platform_identity_changes_environment_id() {
        let graph = {
            let mut nodes = BTreeMap::new();
            nodes.insert(
                PackageKey::root(),
                PackageNode {
                    key: PackageKey::root(),
                    name: Some("demo".into()),
                    version: Some("1.0.0".into()),
                    source: PackageSource::Workspace,
                    integrity: None,
                    dependencies: BTreeMap::new(),
                    dev_dependencies: BTreeMap::new(),
                    optional_dependencies: BTreeMap::new(),
                    peer_dependencies: BTreeMap::new(),
                    peer_dependencies_meta: BTreeMap::new(),
                    has_install_script: false,
                    optional: false,
                    dev: false,
                    peer: false,
                    cpu: Vec::new(),
                    os: Vec::new(),
                    bundled_dependencies: Vec::new(),
                    is_workspace: true,
                    is_link: false,
                    likely_native: false,
                    bin: BTreeMap::new(),
                },
            );
            DependencyGraph {
                lockfile_kind: LockfileKind::NpmPackageLock,
                lockfile_version: 3,
                root: PackageKey::root(),
                nodes,
                edges: Vec::new(),
            }
        };
        let linux = PlatformIdentity {
            os: "linux".into(),
            arch: "x86_64".into(),
        };
        let macos = PlatformIdentity {
            os: "macos".into(),
            arch: "aarch64".into(),
        };
        let a = EnvironmentId::derive(&graph, &linux);
        let b = EnvironmentId::derive(&graph, &macos);
        assert_ne!(a, b);
        assert_eq!(a, EnvironmentId::derive(&graph, &linux));
    }
}
