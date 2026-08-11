//! Artifact acquisition: fetch → verify integrity → store immutably.
//!
//! The registry/transport is behind [`ArtifactSource`] so offline caches,
//! mirrors, and tests can substitute without touching the domain graph.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use weave_core::{
    platform_fit, DependencyGraph, Error, HostPlatform, Integrity, PackageKey, PackageNode,
    PackageSource, PlatformFit,
};
use weave_store::{ArtifactId, ContentStore};

/// Request to fetch one package artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRequest {
    /// Package name for diagnostics.
    pub name: String,
    /// Resolved version when known.
    pub version: Option<String>,
    /// Download URL or local path source.
    pub source: PackageSource,
    /// Optional npm integrity string.
    pub integrity: Option<String>,
    /// Lockfile node key (for reporting).
    pub key: PackageKey,
}

impl ArtifactRequest {
    /// Build a request from a graph node that has a fetchable registry/path source.
    pub fn from_node(node: &PackageNode) -> Option<Self> {
        if node.key.is_root() || node.is_workspace {
            return None;
        }
        match &node.source {
            PackageSource::Registry { .. } | PackageSource::Path { .. } => Some(Self {
                name: node.name.clone().unwrap_or_else(|| node.key.to_string()),
                version: node.version.clone(),
                source: node.source.clone(),
                integrity: node.integrity.clone(),
                key: node.key.clone(),
            }),
            PackageSource::Link { .. } | PackageSource::Workspace => None,
            PackageSource::Other { resolved: Some(r) }
                if r.starts_with("http://") || r.starts_with("https://") =>
            {
                Some(Self {
                    name: node.name.clone().unwrap_or_else(|| node.key.to_string()),
                    version: node.version.clone(),
                    source: PackageSource::Registry {
                        resolved: r.clone(),
                    },
                    integrity: node.integrity.clone(),
                    key: node.key.clone(),
                })
            }
            PackageSource::Other { .. } => None,
        }
    }
}

/// Bytes fetched for an artifact prior to store insertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedArtifact {
    /// Raw artifact bytes (usually a tarball).
    pub bytes: Vec<u8>,
}

/// Pluggable artifact transport.
pub trait ArtifactSource {
    /// Fetch artifact bytes for `request`.
    fn fetch(&self, request: &ArtifactRequest) -> weave_core::Result<FetchedArtifact>;
}

/// HTTP(S) tarball fetcher (npm registry compatible URLs).
#[derive(Debug, Default, Clone)]
pub struct HttpArtifactSource {
    /// Optional user-agent override.
    pub user_agent: String,
}

impl HttpArtifactSource {
    /// Create a fetcher with the Weave user-agent.
    pub fn new() -> Self {
        Self {
            user_agent: format!("weave/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

impl ArtifactSource for HttpArtifactSource {
    fn fetch(&self, request: &ArtifactRequest) -> weave_core::Result<FetchedArtifact> {
        let url = match &request.source {
            PackageSource::Registry { resolved } => resolved.clone(),
            other => {
                return Err(Error::FetchFailed {
                    url: format!("{other:?}"),
                    reason: "HttpArtifactSource only supports registry URLs".into(),
                });
            }
        };

        let response = ureq::get(&url)
            .set("User-Agent", &self.user_agent)
            .call()
            .map_err(|err| Error::FetchFailed {
                url: url.clone(),
                reason: err.to_string(),
            })?;

        let mut bytes = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut bytes)
            .map_err(|err| Error::FetchFailed {
                url: url.clone(),
                reason: err.to_string(),
            })?;

        Ok(FetchedArtifact { bytes })
    }
}

/// Default production source: local `file:` / path snapshots + HTTPS registry.
#[derive(Debug, Clone)]
pub struct DefaultArtifactSource {
    http: HttpArtifactSource,
    files: FileArtifactSource,
}

impl DefaultArtifactSource {
    /// Bind path resolution to a project root (for relative `file:` deps).
    pub fn for_project(project_root: impl Into<PathBuf>) -> Self {
        Self {
            http: HttpArtifactSource::new(),
            files: FileArtifactSource::new(project_root),
        }
    }
}

impl ArtifactSource for DefaultArtifactSource {
    fn fetch(&self, request: &ArtifactRequest) -> weave_core::Result<FetchedArtifact> {
        match &request.source {
            PackageSource::Path { .. } => self.files.fetch(request),
            PackageSource::Registry { .. } => self.http.fetch(request),
            PackageSource::Other { resolved: Some(r) }
                if r.starts_with("http://") || r.starts_with("https://") =>
            {
                self.http.fetch(request)
            }
            other => Err(Error::FetchFailed {
                url: format!("{other:?}"),
                reason: "unsupported artifact source for DefaultArtifactSource".into(),
            }),
        }
    }
}

/// Reads artifacts from local filesystem paths (`file:` / path sources).
#[derive(Debug, Clone)]
pub struct FileArtifactSource {
    /// Directory used to resolve relative path sources.
    pub base_dir: PathBuf,
    /// Optional map from package name → absolute file path (tests / offline).
    pub overrides: BTreeMap<String, PathBuf>,
}

impl FileArtifactSource {
    /// Create a file-backed source rooted at `base_dir`.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
            overrides: BTreeMap::new(),
        }
    }

    /// Bind a package name to an on-disk blob for tests.
    pub fn with_override(mut self, name: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        self.overrides.insert(name.into(), path.into());
        self
    }
}

impl ArtifactSource for FileArtifactSource {
    fn fetch(&self, request: &ArtifactRequest) -> weave_core::Result<FetchedArtifact> {
        if let Some(path) = self.overrides.get(&request.name) {
            let bytes = fs::read(path).map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
            return Ok(FetchedArtifact { bytes });
        }

        let path = match &request.source {
            PackageSource::Path { path } => {
                let p = PathBuf::from(path);
                if p.is_absolute() {
                    p
                } else {
                    self.base_dir.join(p)
                }
            }
            PackageSource::Registry { resolved } => {
                return Err(Error::FetchFailed {
                    url: resolved.clone(),
                    reason: "FileArtifactSource cannot fetch registry URLs".into(),
                });
            }
            other => {
                return Err(Error::FetchFailed {
                    url: format!("{other:?}"),
                    reason: "unsupported source for FileArtifactSource".into(),
                });
            }
        };

        // Directory `file:` deps are snapshotted into an immutable tarball
        // (ADR-0014). Mutable source trees are never stored as live mounts.
        if path.is_dir() {
            let bytes = weave_fs::pack_directory_as_npm_tarball(&path)?;
            return Ok(FetchedArtifact { bytes });
        }

        let bytes = fs::read(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        Ok(FetchedArtifact { bytes })
    }
}

/// Result of preparing one artifact into the content store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredArtifact {
    /// Package request that was acquired.
    pub request: ArtifactRequest,
    /// Content-addressed id in the store.
    pub artifact_id: ArtifactId,
    /// Whether bytes were newly written (`false` = reused existing object).
    pub newly_stored: bool,
}

/// Summary of platform filtering during acquisition.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AcquireFilterReport {
    /// Optional packages skipped due to os/cpu mismatch.
    pub skipped_optional: usize,
    /// Keys of skipped optional packages.
    pub skipped_keys: Vec<String>,
}

/// Acquire all fetchable artifacts for `graph` into `store`.
///
/// Optional packages whose `os`/`cpu` constraints reject the host are skipped.
/// Required packages that reject the host return [`Error::InvalidState`].
pub fn prepare_artifacts<S: ArtifactSource>(
    graph: &DependencyGraph,
    source: &S,
    store: &ContentStore,
) -> weave_core::Result<Vec<StoredArtifact>> {
    Ok(prepare_artifacts_for_platform(graph, source, store, &HostPlatform::current())?.0)
}

/// Like [`prepare_artifacts`] with an explicit host platform (tests / cross-compile).
pub fn prepare_artifacts_for_platform<S: ArtifactSource>(
    graph: &DependencyGraph,
    source: &S,
    store: &ContentStore,
    host: &HostPlatform,
) -> weave_core::Result<(Vec<StoredArtifact>, AcquireFilterReport)> {
    let mut out = Vec::new();
    let mut filter = AcquireFilterReport::default();
    for node in graph.nodes.values() {
        match platform_fit(node, host) {
            PlatformFit::Compatible => {}
            PlatformFit::SkipOptional => {
                filter.skipped_optional += 1;
                filter.skipped_keys.push(node.key.as_str().to_owned());
                continue;
            }
            PlatformFit::RejectRequired => {
                return Err(Error::InvalidState {
                    path: std::path::PathBuf::from(node.key.as_str()),
                    reason: format!(
                        "package {} is incompatible with host {}/{} (os={:?} cpu={:?})",
                        node.name.as_deref().unwrap_or(node.key.as_str()),
                        host.npm_os(),
                        host.npm_cpu(),
                        node.os,
                        node.cpu
                    ),
                });
            }
        }
        let Some(request) = ArtifactRequest::from_node(node) else {
            continue;
        };
        out.push(acquire_one(&request, source, store)?);
    }
    Ok((out, filter))
}

/// Fetch, verify integrity, and store a single artifact.
pub fn acquire_one<S: ArtifactSource>(
    request: &ArtifactRequest,
    source: &S,
    store: &ContentStore,
) -> weave_core::Result<StoredArtifact> {
    // Fast path: if we already have an object keyed by a previous content hash
    // we still must fetch/verify unless we map integrity→artifact. For MVP we
    // fetch (or read) then put idempotently — ContentStore deduplicates bytes.
    let fetched = source.fetch(request)?;

    if let Some(raw) = &request.integrity {
        let integrity = Integrity::parse(raw)?;
        integrity.verify(&fetched.bytes, &request.name)?;
    }

    let id = weave_store::hash_bytes(&fetched.bytes);
    let existed = store.contains(&id);
    if existed {
        store.verify(&id)?;
    } else {
        store.put(&fetched.bytes, Some(&id))?;
    }

    Ok(StoredArtifact {
        request: request.clone(),
        artifact_id: id,
        newly_stored: !existed,
    })
}

/// Convenience: open the default store and prepare artifacts for a lockfile.
pub fn prepare_lockfile_artifacts<S: ArtifactSource>(
    lockfile: &Path,
    source: &S,
    store_root: &Path,
) -> weave_core::Result<Vec<StoredArtifact>> {
    let graph = weave_lockfile::parse_lockfile(lockfile)?;
    let store = ContentStore::open(store_root)?;
    prepare_artifacts(&graph, source, &store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use weave_core::{LockfileKind, PackageKey, PackageNode};

    fn sri_sha256(bytes: &[u8]) -> String {
        let digest = Sha256::digest(bytes);
        format!("sha256-{}", b64(&digest))
    }

    fn b64(bytes: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let a = chunk[0] as u32;
            let b = chunk.get(1).copied().unwrap_or(0) as u32;
            let c = chunk.get(2).copied().unwrap_or(0) as u32;
            let n = (a << 16) | (b << 8) | c;
            out.push(T[((n >> 18) & 63) as usize] as char);
            out.push(T[((n >> 12) & 63) as usize] as char);
            if chunk.len() > 1 {
                out.push(T[((n >> 6) & 63) as usize] as char);
            } else {
                out.push('=');
            }
            if chunk.len() > 2 {
                out.push(T[(n & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
        out
    }

    #[test]
    fn acquires_file_artifact_with_integrity_into_store() {
        let tmp = tempfile::tempdir().unwrap();
        let blob = tmp.path().join("pkg.tgz");
        let data = b"fake-tarball-bytes";
        fs::write(&blob, data).unwrap();

        let source = FileArtifactSource::new(tmp.path()).with_override("demo-pkg", blob.clone());
        let store = ContentStore::open(tmp.path().join("objects")).unwrap();

        let request = ArtifactRequest {
            name: "demo-pkg".into(),
            version: Some("1.0.0".into()),
            source: PackageSource::Path {
                path: "pkg.tgz".into(),
            },
            integrity: Some(sri_sha256(data)),
            key: PackageKey::new("node_modules/demo-pkg"),
        };

        let stored = acquire_one(&request, &source, &store).unwrap();
        assert!(stored.newly_stored);
        assert_eq!(store.get(&stored.artifact_id).unwrap(), data);

        let again = acquire_one(&request, &source, &store).unwrap();
        assert!(!again.newly_stored);
        assert_eq!(again.artifact_id, stored.artifact_id);
    }

    #[test]
    fn rejects_integrity_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let blob = tmp.path().join("pkg.tgz");
        fs::write(&blob, b"actual").unwrap();
        let source = FileArtifactSource::new(tmp.path()).with_override("x", &blob);
        let store = ContentStore::open(tmp.path().join("objects")).unwrap();
        let request = ArtifactRequest {
            name: "x".into(),
            version: None,
            source: PackageSource::Path {
                path: "pkg.tgz".into(),
            },
            integrity: Some(sri_sha256(b"expected")),
            key: PackageKey::new("node_modules/x"),
        };
        let err = acquire_one(&request, &source, &store).unwrap_err();
        assert!(matches!(err, Error::IntegrityCheckFailed { .. }));
        // No final objects should exist — only the empty sha256 root.
        let mut objects = 0;
        for shard in fs::read_dir(store.root().join("sha256")).unwrap() {
            let shard = shard.unwrap().path();
            if shard.is_dir() {
                objects += fs::read_dir(&shard).unwrap().count();
            }
        }
        assert_eq!(objects, 0);
    }

    #[test]
    fn acquires_directory_file_dep_as_immutable_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg = tmp.path().join("vendor/local");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(
            pkg.join("package.json"),
            br#"{"name":"local","version":"1.0.0"}"#,
        )
        .unwrap();
        fs::write(pkg.join("index.js"), b"module.exports=1;\n").unwrap();

        let source = FileArtifactSource::new(tmp.path());
        let store = ContentStore::open(tmp.path().join("objects")).unwrap();
        let request = ArtifactRequest {
            name: "local".into(),
            version: Some("1.0.0".into()),
            source: PackageSource::Path {
                path: "vendor/local".into(),
            },
            integrity: None,
            key: PackageKey::new("node_modules/local"),
        };
        let stored = acquire_one(&request, &source, &store).unwrap();
        assert!(stored.newly_stored);
        // Mutate source — re-acquire same path should hash differently.
        fs::write(pkg.join("index.js"), b"module.exports=2;\n").unwrap();
        let again = acquire_one(&request, &source, &store).unwrap();
        assert_ne!(stored.artifact_id, again.artifact_id);
    }

    #[test]
    fn prepare_artifacts_skips_workspace_links() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::open(tmp.path().join("objects")).unwrap();
        let mut nodes = BTreeMap::new();
        nodes.insert(
            PackageKey::root(),
            PackageNode {
                key: PackageKey::root(),
                name: Some("root".into()),
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
        let source = FileArtifactSource::new(tmp.path());
        let stored = prepare_artifacts(&graph, &source, &store).unwrap();
        assert!(stored.is_empty());
    }
}
