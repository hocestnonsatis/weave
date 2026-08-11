//! Parse npm `package-lock.json` into a [`DependencyGraph`].

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;
use weave_core::{
    resolve_dependency_key, DependencyEdge, DependencyGraph, EdgeKind, Error, LockfileKind,
    PackageKey, PackageNode, PackageSource, PeerMeta,
};

use crate::detect::detect_lockfile;

/// Parse a lockfile on disk into a deterministic dependency graph.
pub fn parse_lockfile(path: &Path) -> weave_core::Result<DependencyGraph> {
    let bytes = fs::read(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_lockfile_bytes(path, &bytes)
}

/// Discover and parse the lockfile under `root`.
pub fn parse_project_lockfile(root: &Path) -> weave_core::Result<DependencyGraph> {
    let info = detect_lockfile(root)?.ok_or_else(|| Error::MissingLockfile {
        root: root.to_path_buf(),
    })?;
    parse_lockfile(&info.path)
}

/// Parse lockfile bytes (path used only for diagnostics).
pub fn parse_lockfile_bytes(path: &Path, bytes: &[u8]) -> weave_core::Result<DependencyGraph> {
    let value: Value = serde_json::from_slice(bytes).map_err(|err| Error::InvalidLockfile {
        path: path.to_path_buf(),
        reason: format!("invalid JSON: {err}"),
    })?;

    let version = value
        .get("lockfileVersion")
        .and_then(Value::as_u64)
        .unwrap_or(1) as u32;

    match version {
        1 => parse_v1(path, &value),
        2 | 3 => parse_packages_format(path, version, &value),
        other => Err(Error::UnsupportedLockfile {
            path: path.to_path_buf(),
            reason: format!("lockfileVersion {other} is not supported yet"),
        }),
    }
}

#[derive(Debug, Deserialize)]
struct PackagesEntry {
    name: Option<String>,
    version: Option<String>,
    resolved: Option<String>,
    integrity: Option<String>,
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
    #[serde(default, rename = "devDependencies")]
    dev_dependencies: BTreeMap<String, String>,
    #[serde(default, rename = "optionalDependencies")]
    optional_dependencies: BTreeMap<String, String>,
    #[serde(default, rename = "peerDependencies")]
    peer_dependencies: BTreeMap<String, String>,
    #[serde(default, rename = "peerDependenciesMeta")]
    peer_dependencies_meta: BTreeMap<String, PeerMetaJson>,
    #[serde(default, rename = "hasInstallScript")]
    has_install_script: bool,
    #[serde(default)]
    optional: bool,
    #[serde(default)]
    dev: bool,
    #[serde(default)]
    peer: bool,
    #[serde(default)]
    link: bool,
    #[serde(default)]
    cpu: StringOrVec,
    #[serde(default)]
    os: StringOrVec,
    #[serde(default, rename = "bundledDependencies")]
    bundled_dependencies: Vec<String>,
    #[serde(default, rename = "bundleDependencies")]
    bundle_dependencies_alias: Vec<String>,
    #[serde(default)]
    bin: BinField,
}

#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum BinField {
    #[default]
    None,
    /// `bin: "./cli.js"` — name inferred from package name.
    String(String),
    /// `bin: { "name": "path" }`
    Map(BTreeMap<String, String>),
}

impl BinField {
    fn into_map(self, package_name: Option<&str>) -> BTreeMap<String, String> {
        match self {
            Self::None => BTreeMap::new(),
            Self::String(path) => {
                let Some(name) = package_name else {
                    return BTreeMap::new();
                };
                let bin_name = name.rsplit('/').next().unwrap_or(name);
                let mut map = BTreeMap::new();
                map.insert(bin_name.to_owned(), path);
                map
            }
            Self::Map(map) => map,
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct PeerMetaJson {
    #[serde(default)]
    optional: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum StringOrVec {
    #[default]
    None,
    One(String),
    Many(Vec<String>),
}

impl StringOrVec {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::None => Vec::new(),
            Self::One(s) => vec![s],
            Self::Many(v) => v,
        }
    }
}

fn parse_packages_format(
    path: &Path,
    version: u32,
    value: &Value,
) -> weave_core::Result<DependencyGraph> {
    let packages = value
        .get("packages")
        .and_then(Value::as_object)
        .ok_or_else(|| Error::InvalidLockfile {
            path: path.to_path_buf(),
            reason: "missing packages object".into(),
        })?;

    let mut nodes = BTreeMap::new();

    for (raw_key, entry_value) in packages {
        let entry: PackagesEntry =
            serde_json::from_value(entry_value.clone()).map_err(|err| Error::InvalidLockfile {
                path: path.to_path_buf(),
                reason: format!("invalid packages[{raw_key:?}] entry: {err}"),
            })?;

        let key = PackageKey::new(raw_key.clone());
        let name = entry.name.clone().or_else(|| infer_name_from_key(raw_key));
        let is_link = entry.link || is_link_resolved(entry.resolved.as_deref());
        let is_workspace = key.is_root()
            || (entry.resolved.is_none() && !raw_key.starts_with("node_modules/"))
            || is_workspace_path(raw_key);
        let source = classify_source(entry.resolved.as_deref(), is_link, is_workspace);
        let mut bundled = entry.bundled_dependencies;
        bundled.extend(entry.bundle_dependencies_alias);
        bundled.sort();
        bundled.dedup();

        let likely_native = looks_native(
            name.as_deref(),
            entry.has_install_script,
            &entry.dependencies,
            &entry.optional_dependencies,
        );

        let peer_meta = entry
            .peer_dependencies_meta
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    PeerMeta {
                        optional: v.optional,
                    },
                )
            })
            .collect();

        let bin = entry.bin.into_map(name.as_deref());
        let node = PackageNode {
            key: key.clone(),
            name,
            version: entry.version,
            source,
            integrity: entry.integrity,
            dependencies: entry.dependencies,
            dev_dependencies: entry.dev_dependencies,
            optional_dependencies: entry.optional_dependencies,
            peer_dependencies: entry.peer_dependencies,
            peer_dependencies_meta: peer_meta,
            has_install_script: entry.has_install_script,
            optional: entry.optional,
            dev: entry.dev,
            peer: entry.peer,
            cpu: entry.cpu.into_vec(),
            os: entry.os.into_vec(),
            bundled_dependencies: bundled,
            is_workspace,
            is_link,
            likely_native,
            bin,
        };
        nodes.insert(key, node);
    }

    if !nodes.contains_key(&PackageKey::root()) {
        return Err(Error::InvalidLockfile {
            path: path.to_path_buf(),
            reason: "packages map is missing the root entry \"\"".into(),
        });
    }

    let edges = build_edges_from_nodes(&nodes);
    Ok(DependencyGraph {
        lockfile_kind: LockfileKind::NpmPackageLock,
        lockfile_version: version,
        root: PackageKey::root(),
        nodes,
        edges,
    })
}

/// Flatten classic lockfileVersion 1 nested `dependencies` trees into path keys.
fn parse_v1(_path: &Path, value: &Value) -> weave_core::Result<DependencyGraph> {
    let root_name = value.get("name").and_then(Value::as_str).map(str::to_owned);
    let root_version = value
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_owned);

    let mut nodes = BTreeMap::new();
    nodes.insert(
        PackageKey::root(),
        PackageNode {
            key: PackageKey::root(),
            name: root_name,
            version: root_version,
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

    if let Some(deps) = value.get("dependencies").and_then(Value::as_object) {
        let mut root_deps = BTreeMap::new();
        for (name, dep_value) in deps {
            root_deps.insert(
                name.clone(),
                dep_value
                    .get("version")
                    .and_then(Value::as_str)
                    .unwrap_or("*")
                    .to_owned(),
            );
            walk_v1_dep(name, dep_value, "node_modules", &mut nodes)?;
        }
        if let Some(root) = nodes.get_mut(&PackageKey::root()) {
            root.dependencies = root_deps;
        }
    }

    let edges = build_edges_from_nodes(&nodes);
    Ok(DependencyGraph {
        lockfile_kind: LockfileKind::NpmPackageLock,
        lockfile_version: 1,
        root: PackageKey::root(),
        nodes,
        edges,
    })
}

fn walk_v1_dep(
    name: &str,
    value: &Value,
    parent_modules: &str,
    nodes: &mut BTreeMap<PackageKey, PackageNode>,
) -> weave_core::Result<()> {
    let key_str = format!("{parent_modules}/{name}");
    let key = PackageKey::new(key_str.clone());
    let version = value
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let resolved = value
        .get("resolved")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let integrity = value
        .get("integrity")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let optional = value
        .get("optional")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let dev = value.get("dev").and_then(Value::as_bool).unwrap_or(false);

    let mut requires = BTreeMap::new();
    if let Some(req) = value.get("requires").and_then(Value::as_object) {
        for (k, v) in req {
            if let Some(s) = v.as_str() {
                requires.insert(k.clone(), s.to_owned());
            }
        }
    }

    let is_link = is_link_resolved(resolved.as_deref());
    let source = classify_source(resolved.as_deref(), is_link, false);
    let likely_native = looks_native(Some(name), false, &requires, &BTreeMap::new());

    let bin = match value.get("bin") {
        Some(Value::String(path)) => {
            let bin_name = name.rsplit('/').next().unwrap_or(name);
            let mut map = BTreeMap::new();
            map.insert(bin_name.to_owned(), path.clone());
            map
        }
        Some(Value::Object(obj)) => {
            let mut map = BTreeMap::new();
            for (k, v) in obj {
                if let Some(path) = v.as_str() {
                    map.insert(k.clone(), path.to_owned());
                }
            }
            map
        }
        _ => BTreeMap::new(),
    };

    nodes.insert(
        key.clone(),
        PackageNode {
            key,
            name: Some(name.to_owned()),
            version,
            source,
            integrity,
            dependencies: requires,
            dev_dependencies: BTreeMap::new(),
            optional_dependencies: BTreeMap::new(),
            peer_dependencies: BTreeMap::new(),
            peer_dependencies_meta: BTreeMap::new(),
            has_install_script: false,
            optional,
            dev,
            peer: false,
            cpu: Vec::new(),
            os: Vec::new(),
            bundled_dependencies: Vec::new(),
            is_workspace: false,
            is_link,
            likely_native,
            bin,
        },
    );

    if let Some(nested) = value.get("dependencies").and_then(Value::as_object) {
        let nested_parent = format!("{key_str}/node_modules");
        for (child_name, child_value) in nested {
            walk_v1_dep(child_name, child_value, &nested_parent, nodes)?;
        }
    }

    Ok(())
}

fn build_edges_from_nodes(nodes: &BTreeMap<PackageKey, PackageNode>) -> Vec<DependencyEdge> {
    let mut edges = Vec::new();

    for parent in nodes.values() {
        push_edges(
            &mut edges,
            parent,
            &parent.dependencies,
            EdgeKind::Runtime,
            nodes,
        );
        push_edges(
            &mut edges,
            parent,
            &parent.dev_dependencies,
            EdgeKind::Dev,
            nodes,
        );
        push_edges(
            &mut edges,
            parent,
            &parent.optional_dependencies,
            EdgeKind::Optional,
            nodes,
        );
        // Peer edges: record declared peer names when a matching installed node
        // is reachable via Node resolution from the parent path.
        for name in parent.peer_dependencies.keys() {
            if let Some(child_key) = resolve_dependency_key(&parent.key, name, nodes) {
                edges.push(DependencyEdge {
                    from: parent.key.clone(),
                    to: child_key,
                    name: name.clone(),
                    kind: EdgeKind::Peer,
                });
            }
        }
    }

    edges.sort();
    edges.dedup();
    edges
}

fn push_edges(
    edges: &mut Vec<DependencyEdge>,
    parent: &PackageNode,
    deps: &BTreeMap<String, String>,
    kind: EdgeKind,
    nodes: &BTreeMap<PackageKey, PackageNode>,
) {
    for name in deps.keys() {
        if let Some(child_key) = resolve_dependency_key(&parent.key, name, nodes) {
            edges.push(DependencyEdge {
                from: parent.key.clone(),
                to: child_key,
                name: name.clone(),
                kind,
            });
        }
    }
}

fn infer_name_from_key(key: &str) -> Option<String> {
    if key.is_empty() {
        return None;
    }
    let name = key.rsplit_once("node_modules/").map(|(_, n)| n)?;
    if name.is_empty() {
        return None;
    }
    Some(name.to_owned())
}

fn is_workspace_path(key: &str) -> bool {
    !key.is_empty() && !key.contains("node_modules/")
}

fn is_link_resolved(resolved: Option<&str>) -> bool {
    match resolved {
        Some(r) => r.starts_with("link:") || r.starts_with("file:"),
        None => false,
    }
}

fn classify_source(resolved: Option<&str>, is_link: bool, is_workspace: bool) -> PackageSource {
    match resolved {
        Some(r) if r.starts_with("link:") => PackageSource::Link {
            target: r.trim_start_matches("link:").to_owned(),
        },
        Some(r) if r.starts_with("file:") => PackageSource::Path {
            path: r.trim_start_matches("file:").to_owned(),
        },
        Some(r) if r.starts_with("http://") || r.starts_with("https://") => {
            PackageSource::Registry {
                resolved: r.to_owned(),
            }
        }
        Some(r) if is_link => PackageSource::Link {
            target: r.to_owned(),
        },
        Some(r) => PackageSource::Other {
            resolved: Some(r.to_owned()),
        },
        None if is_workspace => PackageSource::Workspace,
        None => PackageSource::Other { resolved: None },
    }
}

fn looks_native(
    name: Option<&str>,
    has_install_script: bool,
    deps: &BTreeMap<String, String>,
    optional_deps: &BTreeMap<String, String>,
) -> bool {
    let native_markers = [
        "node-gyp",
        "node-addon-api",
        "nan",
        "prebuild-install",
        "node-pre-gyp",
        "@napi-rs/",
        "bindings",
    ];
    let name_hit = name.is_some_and(|n| {
        n.contains("node-gyp")
            || n.ends_with("-native")
            || n.contains("fsevents")
            || n.contains("sharp")
            || n.contains("sqlite3")
            || n.contains("bcrypt")
    });
    let dep_hit = deps
        .keys()
        .chain(optional_deps.keys())
        .any(|d| native_markers.iter().any(|m| d.contains(m)));
    name_hit || dep_hit || (has_install_script && name.is_some_and(|n| n.contains("node")))
}
