//! Materialization plan derived from a dependency graph.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use weave_core::{
    platform_fit, DependencyGraph, HostPlatform, PackageKey, PackageSource, PlatformFit,
};
use weave_store::ArtifactId;

/// One package to place into the filesystem tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedPackage {
    /// Install path key (`node_modules/…`).
    pub key: PackageKey,
    /// Package name when known.
    pub name: Option<String>,
    /// Content-addressed tarball to extract, when applicable.
    pub artifact_id: Option<ArtifactId>,
    /// True when this is a link/workspace package (no tarball extraction).
    pub link_only: bool,
    /// When true, copy files instead of hardlinking (install scripts / native).
    pub prefer_copy: bool,
    /// Bin map (`name` → path relative to package root).
    #[serde(default)]
    pub bins: BTreeMap<String, String>,
    /// Local workspace/link target relative to the project root (when link-only).
    #[serde(default)]
    pub link_target: Option<String>,
    /// True when this package looks native (needs rebuild guidance).
    #[serde(default)]
    pub likely_native: bool,
    /// True when install scripts are declared.
    #[serde(default)]
    pub has_install_script: bool,
}

/// Plan describing how to build a candidate `node_modules` tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializationPlan {
    /// Planned packages sorted by key.
    pub packages: Vec<PlannedPackage>,
    /// Optional packages omitted due to os/cpu mismatch.
    #[serde(default)]
    pub skipped_optional_platform: Vec<String>,
}

impl MaterializationPlan {
    /// Build a plan for the current host platform.
    pub fn from_graph(
        graph: &DependencyGraph,
        artifacts: &BTreeMap<PackageKey, ArtifactId>,
    ) -> Self {
        Self::from_graph_for_platform(graph, artifacts, &HostPlatform::current())
    }

    /// Build a plan from a graph, artifacts, and explicit host platform.
    pub fn from_graph_for_platform(
        graph: &DependencyGraph,
        artifacts: &BTreeMap<PackageKey, ArtifactId>,
        host: &HostPlatform,
    ) -> Self {
        let mut packages = Vec::new();
        let mut skipped_optional_platform = Vec::new();
        for (key, node) in &graph.nodes {
            if key.is_root() {
                continue;
            }
            if node.is_workspace && !key.as_str().starts_with("node_modules/") {
                continue;
            }

            match platform_fit(node, host) {
                PlatformFit::Compatible => {}
                PlatformFit::SkipOptional => {
                    skipped_optional_platform.push(key.as_str().to_owned());
                    continue;
                }
                PlatformFit::RejectRequired => {
                    // Acquire should have failed already; omit from plan as a
                    // defensive measure so we never materialize a bad tree.
                    skipped_optional_platform.push(key.as_str().to_owned());
                    continue;
                }
            }

            let link_only = match &node.source {
                PackageSource::Link { .. } | PackageSource::Workspace => true,
                PackageSource::Path { .. } => false,
                _ => node.is_link,
            };

            let link_target = match &node.source {
                PackageSource::Link { target } => Some(strip_file_or_link_prefix(target)),
                _ => None,
            };

            packages.push(PlannedPackage {
                key: key.clone(),
                name: node.name.clone(),
                artifact_id: artifacts.get(key).cloned(),
                link_only,
                prefer_copy: node.has_install_script || node.likely_native,
                bins: node.bin.clone(),
                link_target,
                likely_native: node.likely_native,
                has_install_script: node.has_install_script,
            });
        }
        packages.sort_by(|a, b| a.key.cmp(&b.key));
        skipped_optional_platform.sort();
        Self {
            packages,
            skipped_optional_platform,
        }
    }
}

fn strip_file_or_link_prefix(raw: &str) -> String {
    let trimmed = raw
        .trim_start_matches("file:")
        .trim_start_matches("link:")
        .trim_start_matches("./");
    trimmed.to_owned()
}
