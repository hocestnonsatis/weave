//! Dependency graph domain models.
//!
//! The graph is the principal dependency identity for Weave environments.
//! Package name+version alone is not sufficient; resolved source and integrity
//! participate in identity when present.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Stable key for a node in the materialized dependency tree.
///
/// For npm lockfile v2/v3 this is typically the `packages` map key
/// (e.g. `""`, `node_modules/lodash`, `node_modules/a/node_modules/b`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PackageKey(String);

impl PackageKey {
    /// Create a package key from a lockfile path string.
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// Root project key (`""` in npm lockfiles).
    pub fn root() -> Self {
        Self::new("")
    }

    /// Whether this key represents the project root.
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Borrow the raw key string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PackageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            f.write_str("<root>")
        } else {
            f.write_str(&self.0)
        }
    }
}

/// How a package artifact was obtained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PackageSource {
    /// Registry tarball URL (or equivalent remote URL).
    Registry {
        /// Resolved download URL.
        resolved: String,
    },
    /// Local `file:` / path dependency.
    Path {
        /// Path as recorded in the lockfile.
        path: String,
    },
    /// `link:` / workspace link without a fetched tarball.
    Link {
        /// Link target as recorded in the lockfile.
        target: String,
    },
    /// Workspace / root package with no external resolution.
    Workspace,
    /// Source could not be classified; keep raw resolved string if any.
    Other {
        /// Optional resolved field.
        resolved: Option<String>,
    },
}

/// Declared relationship from one package to another by dependency name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyRef {
    /// Dependency name as declared in package.json / lockfile.
    pub name: String,
    /// Version range or exact version string from the lockfile edge.
    pub requested: Option<String>,
}

/// A single package node in the dependency graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageNode {
    /// Lockfile path key.
    pub key: PackageKey,
    /// Package name when known.
    pub name: Option<String>,
    /// Resolved version when known.
    pub version: Option<String>,
    /// Where the artifact comes from.
    pub source: PackageSource,
    /// sri / integrity string when present.
    pub integrity: Option<String>,
    /// Production/runtime dependencies.
    pub dependencies: BTreeMap<String, String>,
    /// Development dependencies (typically only on the root).
    pub dev_dependencies: BTreeMap<String, String>,
    /// Optional dependencies.
    pub optional_dependencies: BTreeMap<String, String>,
    /// Peer dependency requirements (name → range).
    pub peer_dependencies: BTreeMap<String, String>,
    /// Peer dependencies marked optional in modern lockfiles.
    pub peer_dependencies_meta: BTreeMap<String, PeerMeta>,
    /// Whether the package declares install/lifecycle scripts.
    pub has_install_script: bool,
    /// Whether this node is marked optional in the lockfile.
    pub optional: bool,
    /// Whether this node is a development-only dependency.
    pub dev: bool,
    /// Whether this is a peer dependency installation.
    pub peer: bool,
    /// CPU constraints from the package (`cpu` field).
    pub cpu: Vec<String>,
    /// OS constraints from the package (`os` field).
    pub os: Vec<String>,
    /// Bundled dependency names when listed.
    pub bundled_dependencies: Vec<String>,
    /// True when the lockfile marks this as a workspace/link package.
    pub is_workspace: bool,
    /// True when `resolved` / link indicates a symlink-style local package.
    pub is_link: bool,
    /// True when the package looks like a native addon (heuristic for MVP).
    pub likely_native: bool,
    /// Executable bin map (`name` → path relative to the package root).
    ///
    /// Populated from the lockfile `bin` field when present. Missing entries
    /// may still be discovered from `package.json` after extraction.
    pub bin: BTreeMap<String, String>,
}

/// Peer dependency metadata flags.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PeerMeta {
    /// Peer may be omitted.
    pub optional: bool,
}

/// Directed edge between two package nodes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DependencyEdge {
    /// Parent package key.
    pub from: PackageKey,
    /// Child package key (resolved install path).
    pub to: PackageKey,
    /// Declared dependency name.
    pub name: String,
    /// Kind of dependency edge.
    pub kind: EdgeKind,
}

/// Classification of a dependency edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeKind {
    /// Normal runtime dependency.
    Runtime,
    /// Development dependency.
    Dev,
    /// Optional dependency.
    Optional,
    /// Peer dependency relationship recorded in the graph.
    Peer,
}

/// Deterministic dependency graph extracted from a lockfile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyGraph {
    /// Lockfile kind label for diagnostics.
    pub lockfile_kind: crate::LockfileKind,
    /// npm lockfileVersion.
    pub lockfile_version: u32,
    /// Root package key.
    pub root: PackageKey,
    /// All package nodes keyed by install path.
    pub nodes: BTreeMap<PackageKey, PackageNode>,
    /// Directed dependency edges (sorted for determinism).
    pub edges: Vec<DependencyEdge>,
}

impl DependencyGraph {
    /// Number of non-root package nodes.
    pub fn package_count(&self) -> usize {
        self.nodes.values().filter(|n| !n.key.is_root()).count()
    }

    /// Return a node by key.
    pub fn get(&self, key: &PackageKey) -> Option<&PackageNode> {
        self.nodes.get(key)
    }

    /// Compute a stable content identity for this graph.
    ///
    /// The identity covers package keys, versions, integrity/resolved sources,
    /// and edge structure — enough to distinguish environments.
    pub fn identity(&self) -> GraphIdentity {
        let mut hasher = Sha256::new();
        hasher.update(b"weave-graph-v1\0");
        hasher.update(self.lockfile_version.to_string().as_bytes());
        hasher.update(b"\0");

        for (key, node) in &self.nodes {
            hasher.update(key.as_str().as_bytes());
            hasher.update(b"\0");
            if let Some(name) = &node.name {
                hasher.update(name.as_bytes());
            }
            hasher.update(b"\0");
            if let Some(version) = &node.version {
                hasher.update(version.as_bytes());
            }
            hasher.update(b"\0");
            if let Some(integrity) = &node.integrity {
                hasher.update(integrity.as_bytes());
            }
            hasher.update(b"\0");
            match &node.source {
                PackageSource::Registry { resolved } => {
                    hasher.update(b"registry\0");
                    hasher.update(resolved.as_bytes());
                }
                PackageSource::Path { path } => {
                    hasher.update(b"path\0");
                    hasher.update(path.as_bytes());
                }
                PackageSource::Link { target } => {
                    hasher.update(b"link\0");
                    hasher.update(target.as_bytes());
                }
                PackageSource::Workspace => hasher.update(b"workspace\0"),
                PackageSource::Other { resolved } => {
                    hasher.update(b"other\0");
                    if let Some(r) = resolved {
                        hasher.update(r.as_bytes());
                    }
                }
            }
            hasher.update(b"\0");
            hasher.update([u8::from(node.has_install_script)]);
            hasher.update([u8::from(node.optional)]);
            hasher.update([u8::from(node.dev)]);
            hasher.update([u8::from(node.peer)]);
            for cpu in &node.cpu {
                hasher.update(cpu.as_bytes());
                hasher.update(b",");
            }
            hasher.update(b"\0");
            for os in &node.os {
                hasher.update(os.as_bytes());
                hasher.update(b",");
            }
            hasher.update(b"\n");
        }

        for edge in &self.edges {
            hasher.update(edge.from.as_str().as_bytes());
            hasher.update(b"->");
            hasher.update(edge.to.as_str().as_bytes());
            hasher.update(b":");
            hasher.update(edge.name.as_bytes());
            hasher.update(format!("{:?}", edge.kind).as_bytes());
            hasher.update(b"\n");
        }

        let digest = hasher.finalize();
        GraphIdentity(hex_encode(&digest))
    }

    /// Names of packages that declare install scripts.
    pub fn packages_with_install_scripts(&self) -> BTreeSet<String> {
        self.nodes
            .values()
            .filter(|n| n.has_install_script)
            .filter_map(|n| n.name.clone())
            .collect()
    }

    /// Resolve `name` the way Node walks `node_modules` from `from`.
    pub fn resolve_dependency(&self, from: &PackageKey, name: &str) -> Option<PackageKey> {
        resolve_dependency_key(from, name, &self.nodes)
    }

    /// Audit peerDependencies against the lockfile install graph.
    ///
    /// Weave does not invent peer installs. Required peers must already appear
    /// in the lockfile at a Node-resolvable path. Optional peers
    /// (`peerDependenciesMeta.*.optional`) may be missing.
    pub fn audit_peers(&self) -> Vec<PeerAuditFinding> {
        let mut findings = Vec::new();
        for node in self.nodes.values() {
            if node.key.is_root() || node.peer_dependencies.is_empty() {
                continue;
            }
            let pkg_label = node
                .name
                .clone()
                .unwrap_or_else(|| node.key.as_str().to_owned());
            for (peer_name, range) in &node.peer_dependencies {
                let optional = node
                    .peer_dependencies_meta
                    .get(peer_name)
                    .map(|m| m.optional)
                    .unwrap_or(false);
                match self.resolve_dependency(&node.key, peer_name) {
                    Some(key) => {
                        let version = self
                            .get(&key)
                            .and_then(|n| n.version.clone())
                            .unwrap_or_default();
                        findings.push(PeerAuditFinding {
                            package: pkg_label.clone(),
                            package_key: node.key.as_str().to_owned(),
                            peer: peer_name.clone(),
                            requested: range.clone(),
                            optional,
                            status: PeerAuditStatus::Satisfied {
                                resolved_key: key.as_str().to_owned(),
                                resolved_version: version,
                            },
                        });
                    }
                    None if optional => {
                        findings.push(PeerAuditFinding {
                            package: pkg_label.clone(),
                            package_key: node.key.as_str().to_owned(),
                            peer: peer_name.clone(),
                            requested: range.clone(),
                            optional: true,
                            status: PeerAuditStatus::MissingOptional,
                        });
                    }
                    None => {
                        findings.push(PeerAuditFinding {
                            package: pkg_label.clone(),
                            package_key: node.key.as_str().to_owned(),
                            peer: peer_name.clone(),
                            requested: range.clone(),
                            optional: false,
                            status: PeerAuditStatus::MissingRequired,
                        });
                    }
                }
            }
        }
        findings.sort_by(|a, b| (&a.package_key, &a.peer).cmp(&(&b.package_key, &b.peer)));
        findings
    }
}

/// One peer-dependency audit result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerAuditFinding {
    /// Package that declared the peer.
    pub package: String,
    /// Lockfile key of the declaring package.
    pub package_key: String,
    /// Peer package name.
    pub peer: String,
    /// Requested range from the lockfile.
    pub requested: String,
    /// Whether the peer is marked optional.
    pub optional: bool,
    /// Satisfaction status.
    pub status: PeerAuditStatus,
}

/// Peer satisfaction status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PeerAuditStatus {
    /// Peer is present at a Node-resolvable path.
    Satisfied {
        /// Resolved install key.
        resolved_key: String,
        /// Resolved version when known.
        resolved_version: String,
    },
    /// Required peer is absent from the install graph.
    MissingRequired,
    /// Optional peer is absent (allowed).
    MissingOptional,
}

/// Resolve `name` the way Node walks `node_modules` from `from`'s directory.
pub fn resolve_dependency_key(
    from: &PackageKey,
    name: &str,
    nodes: &BTreeMap<PackageKey, PackageNode>,
) -> Option<PackageKey> {
    let start = if from.is_root() {
        String::new()
    } else {
        from.as_str().to_owned()
    };

    let mut dirs = Vec::new();
    if start.is_empty() {
        dirs.push(String::new());
    } else {
        let mut current = start;
        loop {
            dirs.push(current.clone());
            match current.rsplit_once("/node_modules") {
                Some((prefix, _)) => {
                    if prefix.is_empty() {
                        dirs.push(String::new());
                        break;
                    }
                    current = prefix.to_owned();
                }
                None => {
                    dirs.push(String::new());
                    break;
                }
            }
        }
    }

    for dir in dirs {
        let candidate = if dir.is_empty() {
            PackageKey::new(format!("node_modules/{name}"))
        } else {
            PackageKey::new(format!("{dir}/node_modules/{name}"))
        };
        if nodes.contains_key(&candidate) {
            return Some(candidate);
        }
    }

    for (key, node) in nodes {
        if node.is_workspace && node.name.as_deref() == Some(name) && !key.is_root() {
            return Some(key.clone());
        }
    }

    None
}

/// Hex-encoded SHA-256 identity of a [`DependencyGraph`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GraphIdentity(String);

impl GraphIdentity {
    /// Borrow the hex digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for GraphIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn hex_encode(bytes: &[u8]) -> String {
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

    fn empty_node(key: &str) -> PackageNode {
        PackageNode {
            key: PackageKey::new(key),
            name: None,
            version: None,
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
            is_workspace: key.is_empty(),
            is_link: false,
            likely_native: false,
            bin: BTreeMap::new(),
        }
    }

    #[test]
    fn identity_is_stable_and_order_independent_for_same_nodes() {
        let mut a = DependencyGraph {
            lockfile_kind: crate::LockfileKind::NpmPackageLock,
            lockfile_version: 3,
            root: PackageKey::root(),
            nodes: BTreeMap::new(),
            edges: Vec::new(),
        };
        a.nodes.insert(PackageKey::root(), empty_node(""));
        a.nodes.insert(
            PackageKey::new("node_modules/left-pad"),
            PackageNode {
                name: Some("left-pad".into()),
                version: Some("1.3.0".into()),
                integrity: Some("sha512-abc".into()),
                source: PackageSource::Registry {
                    resolved: "https://example/left-pad.tgz".into(),
                },
                ..empty_node("node_modules/left-pad")
            },
        );

        let id1 = a.identity();
        let id2 = a.identity();
        assert_eq!(id1, id2);
        assert_eq!(id1.as_str().len(), 64);
    }
}
