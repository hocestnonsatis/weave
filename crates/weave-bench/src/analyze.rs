//! Graph / lockfile scale metrics for Phase 3.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use serde::Serialize;
use weave_core::{DependencyGraph, PackageKey, PackageSource};
use weave_lockfile::parse_lockfile;

/// Structural metrics extracted from a lockfile graph (no network).
#[derive(Debug, Clone, Serialize)]
pub struct GraphStats {
    pub path: String,
    pub lockfile_version: u32,
    pub package_count: usize,
    pub unique_artifacts: usize,
    pub unique_name_version: usize,
    pub duplicated_name_count: usize,
    pub max_depth: usize,
    pub mean_depth: f64,
    pub optional_packages: usize,
    pub peer_edges: usize,
    pub peer_packages: usize,
    pub native_packages: usize,
    pub lifecycle_script_packages: usize,
    pub workspace_packages: usize,
    pub link_packages: usize,
    pub registry_packages: usize,
    pub path_packages: usize,
    pub packages_with_cpu_os: usize,
    pub graph_identity_prefix: String,
    /// Integrity values present (artifact fingerprints).
    pub integrity_present: usize,
    pub integrity_missing_registry: usize,
}

/// Analyze a lockfile path.
pub fn analyze_lockfile(path: &Path) -> anyhow::Result<GraphStats> {
    let graph = parse_lockfile(path).map_err(anyhow::Error::msg)?;
    Ok(stats_from_graph(path, &graph))
}

/// Compute stats from an already-parsed graph.
pub fn stats_from_graph(path: &Path, graph: &DependencyGraph) -> GraphStats {
    let mut artifact_ids = BTreeSet::new();
    let mut name_versions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut optional = 0usize;
    let mut peer_pkgs = 0usize;
    let mut native = 0usize;
    let mut lifecycle = 0usize;
    let mut workspace = 0usize;
    let mut link = 0usize;
    let mut registry = 0usize;
    let mut path_pkgs = 0usize;
    let mut cpu_os = 0usize;
    let mut integrity_present = 0usize;
    let mut integrity_missing_registry = 0usize;

    for node in graph.nodes.values() {
        if node.key.is_root() {
            continue;
        }
        if let (Some(name), Some(ver)) = (&node.name, &node.version) {
            name_versions
                .entry(name.clone())
                .or_default()
                .insert(ver.clone());
        }
        if node.optional {
            optional += 1;
        }
        if node.peer {
            peer_pkgs += 1;
        }
        if node.likely_native {
            native += 1;
        }
        if node.has_install_script {
            lifecycle += 1;
        }
        if node.is_workspace {
            workspace += 1;
        }
        if node.is_link {
            link += 1;
        }
        if !node.cpu.is_empty() || !node.os.is_empty() {
            cpu_os += 1;
        }
        match &node.source {
            PackageSource::Registry { resolved } => {
                registry += 1;
                if let Some(int) = &node.integrity {
                    integrity_present += 1;
                    artifact_ids.insert(format!("{resolved}|{int}"));
                } else {
                    integrity_missing_registry += 1;
                    artifact_ids.insert(resolved.clone());
                }
            }
            PackageSource::Path { .. } => path_pkgs += 1,
            PackageSource::Link { .. } => {}
            PackageSource::Workspace => {}
            PackageSource::Other { resolved } => {
                if let Some(r) = resolved {
                    artifact_ids.insert(r.clone());
                }
            }
        }
    }

    let duplicated_name_count = name_versions.values().filter(|vers| vers.len() > 1).count();
    let unique_name_version: usize = name_versions.values().map(|v| v.len()).sum();

    let peer_edges = graph
        .edges
        .iter()
        .filter(|e| matches!(e.kind, weave_core::EdgeKind::Peer))
        .count();

    let depths = node_depths(graph);
    let max_depth = depths.values().copied().max().unwrap_or(0);
    let mean_depth = if depths.is_empty() {
        0.0
    } else {
        depths.values().sum::<usize>() as f64 / depths.len() as f64
    };

    let id = graph.identity();
    let prefix: String = id.as_str().chars().take(16).collect();

    GraphStats {
        path: path.display().to_string(),
        lockfile_version: graph.lockfile_version,
        package_count: graph.package_count(),
        unique_artifacts: artifact_ids.len(),
        unique_name_version,
        duplicated_name_count,
        max_depth,
        mean_depth,
        optional_packages: optional,
        peer_edges,
        peer_packages: peer_pkgs,
        native_packages: native,
        lifecycle_script_packages: lifecycle,
        workspace_packages: workspace,
        link_packages: link,
        registry_packages: registry,
        path_packages: path_pkgs,
        packages_with_cpu_os: cpu_os,
        graph_identity_prefix: prefix,
        integrity_present,
        integrity_missing_registry,
    }
}

/// BFS depth from root along dependency edges.
fn node_depths(graph: &DependencyGraph) -> BTreeMap<PackageKey, usize> {
    let mut depths = BTreeMap::new();
    let mut q = VecDeque::new();
    depths.insert(graph.root.clone(), 0);
    q.push_back(graph.root.clone());
    while let Some(cur) = q.pop_front() {
        let d = depths[&cur];
        for edge in graph.edges.iter().filter(|e| e.from == cur) {
            depths.entry(edge.to.clone()).or_insert_with(|| {
                q.push_back(edge.to.clone());
                d + 1
            });
        }
    }
    depths
}

/// Artifact fingerprint set (resolved|integrity) for overlap analysis.
pub fn artifact_set(graph: &DependencyGraph) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for node in graph.nodes.values() {
        if node.key.is_root() {
            continue;
        }
        if let PackageSource::Registry { resolved } = &node.source {
            match &node.integrity {
                Some(i) => out.insert(format!("{resolved}|{i}")),
                None => out.insert(resolved.clone()),
            };
        }
    }
    out
}

/// Jaccard / shared fraction of artifact sets.
#[derive(Debug, Clone, Serialize)]
pub struct OverlapReport {
    pub a_artifacts: usize,
    pub b_artifacts: usize,
    pub shared: usize,
    pub only_a: usize,
    pub only_b: usize,
    /// shared / |A ∪ B|
    pub jaccard: f64,
    /// shared / |A| (reuse relative to A)
    pub shared_fraction_of_a: f64,
    /// shared / |B|
    pub shared_fraction_of_b: f64,
}

pub fn overlap(a: &BTreeSet<String>, b: &BTreeSet<String>) -> OverlapReport {
    let shared = a.intersection(b).count();
    let only_a = a.difference(b).count();
    let only_b = b.difference(a).count();
    let union = shared + only_a + only_b;
    OverlapReport {
        a_artifacts: a.len(),
        b_artifacts: b.len(),
        shared,
        only_a,
        only_b,
        jaccard: if union == 0 {
            0.0
        } else {
            shared as f64 / union as f64
        },
        shared_fraction_of_a: if a.is_empty() {
            0.0
        } else {
            shared as f64 / a.len() as f64
        },
        shared_fraction_of_b: if b.is_empty() {
            0.0
        } else {
            shared as f64 / b.len() as f64
        },
    }
}

/// Classify a package node for lifecycle experiment (no execution).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum LifecycleClass {
    ExtractionOnly,
    LikelyGeneratedFiles,
    LikelyNativeBuild,
    RuntimeInstallRequired,
    UnsupportedOrUnsafe,
}

pub fn classify_node(node: &weave_core::PackageNode) -> LifecycleClass {
    if node.key.is_root() || node.is_link || node.is_workspace {
        return LifecycleClass::ExtractionOnly;
    }
    if node.likely_native && node.has_install_script {
        return LifecycleClass::LikelyNativeBuild;
    }
    if node.likely_native {
        return LifecycleClass::LikelyNativeBuild;
    }
    if node.has_install_script {
        // Heuristic: install script without native markers → generated files or runtime install
        if node
            .name
            .as_deref()
            .is_some_and(|n| n.contains("esbuild") || n.contains("swc") || n.contains("protobuf"))
        {
            return LifecycleClass::LikelyGeneratedFiles;
        }
        return LifecycleClass::RuntimeInstallRequired;
    }
    LifecycleClass::ExtractionOnly
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleSummary {
    pub path: String,
    pub extraction_only: usize,
    pub likely_generated_files: usize,
    pub likely_native_build: usize,
    pub runtime_install_required: usize,
    pub unsupported_or_unsafe: usize,
    pub sample_runtime_install: Vec<String>,
    pub sample_native: Vec<String>,
}

pub fn classify_lockfile(path: &Path) -> anyhow::Result<LifecycleSummary> {
    let graph = parse_lockfile(path).map_err(anyhow::Error::msg)?;
    let mut summary = LifecycleSummary {
        path: path.display().to_string(),
        extraction_only: 0,
        likely_generated_files: 0,
        likely_native_build: 0,
        runtime_install_required: 0,
        unsupported_or_unsafe: 0,
        sample_runtime_install: Vec::new(),
        sample_native: Vec::new(),
    };
    for node in graph.nodes.values() {
        if node.key.is_root() {
            continue;
        }
        match classify_node(node) {
            LifecycleClass::ExtractionOnly => summary.extraction_only += 1,
            LifecycleClass::LikelyGeneratedFiles => summary.likely_generated_files += 1,
            LifecycleClass::LikelyNativeBuild => {
                summary.likely_native_build += 1;
                if summary.sample_native.len() < 8 {
                    if let Some(n) = &node.name {
                        summary.sample_native.push(n.clone());
                    }
                }
            }
            LifecycleClass::RuntimeInstallRequired => {
                summary.runtime_install_required += 1;
                if summary.sample_runtime_install.len() < 8 {
                    if let Some(n) = &node.name {
                        summary.sample_runtime_install.push(n.clone());
                    }
                }
            }
            LifecycleClass::UnsupportedOrUnsafe => summary.unsupported_or_unsafe += 1,
        }
    }
    Ok(summary)
}
