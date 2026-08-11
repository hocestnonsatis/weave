//! Non-executing lifecycle/native execution planning (ADR-0018).
//!
//! Builds a dry-run plan of what *would* execute under the sandboxed execution
//! model. This module never spawns scripts or mutates package trees.
//!
//! Phase 9 enriches plans with package-metadata discovery (scripts + candidate
//! outputs) while keeping a hard distinction between **discovered candidates**
//! and **config-allowed** outputs.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use weave_core::{DependencyGraph, PackageNode};

use crate::config::ExecutionConfig;
use crate::exec_discover::{
    discover_package_dir, resolve_package_dir_for_discovery, DiscoveredScript, OutputCandidate,
    PackageDiscovery, PolicyReviewStatus,
};
use crate::prebuild_fetch::{plan_prebuild_for_package, PrebuildPlanEntry};
use crate::prebuild_resolve::{resolve_native_prebuilds_at, NativePrebuildRequirement};

/// Why a package is considered for controlled execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecNeedClass {
    /// Tarball contents are expected to be enough.
    ExtractionOnly,
    /// Install scripts likely generate runtime files.
    GeneratedFiles,
    /// Native compile / prebuild fetch likely required.
    NativeBuild,
    /// Broader runtime install mutation (reserved; heuristics TBD).
    #[allow(dead_code)]
    RuntimeInstall,
    /// Must never be auto-executed.
    UnsupportedUnsafe,
}

/// Sandbox profile a future runner would request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecSandboxProfile {
    /// No network; default for ADR-0018 v1.
    Offline,
    /// Allowlisted prebuild CDN access only.
    PrebuildFetch,
}

/// One package entry in an execution plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecPlanEntry {
    /// Lockfile package key.
    pub package_key: String,
    /// Package name when known.
    pub name: Option<String>,
    /// Classification.
    pub class: ExecNeedClass,
    /// Whether the package appears to need opt-in execution (pre-policy).
    pub needs_execution: bool,
    /// Whether current config would allow running under `--with-exec`.
    ///
    /// False when not allowlisted / blocked — never implies plain `switch` runs.
    pub would_execute: bool,
    /// Lifecycle script names that would be candidates (lockfile heuristic).
    pub candidate_scripts: Vec<String>,
    /// Scripts discovered from package.json (empty if metadata unavailable).
    pub discovered_scripts: Vec<DiscoveredScript>,
    /// Discovered output path candidates (not approvals).
    pub discovered_output_candidates: Vec<OutputCandidate>,
    /// Outputs currently declared in config for this package.
    pub allowed_outputs: Vec<String>,
    /// Whether the package name appears in `execution.allow_packages`.
    pub package_allowed: bool,
    /// Policy review status vs current config.
    pub policy: PolicyReviewStatus,
    /// True when package.json / static metadata was loaded from disk.
    pub metadata_loaded: bool,
    /// Suggested sandbox profile.
    pub sandbox: ExecSandboxProfile,
    /// Whether a selected prebuild fetch would require network under current profile.
    pub needs_network: bool,
    /// Dry-run prebuild fetch plan entries (no network access).
    pub prebuild: Vec<PrebuildPlanEntry>,
    /// Native prebuild resolution diagnostics (Phase 11; no network).
    pub native_prebuilds: Vec<NativePrebuildRequirement>,
    /// Human-readable rationale (why execution is needed / blocked).
    pub reason: String,
}

/// Full dry-run plan for a dependency graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecPlan {
    /// Project root when known.
    pub root: Option<String>,
    /// Entries sorted by package key.
    pub entries: Vec<ExecPlanEntry>,
    /// Count with `needs_execution` (discovery/classification).
    pub needs_execution_count: usize,
    /// Count that would execute under current allowlist policy.
    pub would_execute_count: usize,
    /// Count still needing human review before allowlisting.
    pub needs_review_count: usize,
    /// Count of packages with selected prebuild network requirements.
    pub needs_network_count: usize,
    /// Count of packages needing manual native prebuild policy.
    pub native_policy_gap_count: usize,
    /// Reminder: no scripts were run building this plan.
    pub executed: bool,
}

impl ExecPlan {
    /// True when any package would require opt-in execution for completeness.
    pub fn needs_opt_in_execution(&self) -> bool {
        self.needs_execution_count > 0
    }
}

/// Build a non-executing plan for `graph` (lockfile heuristics only).
pub fn plan_execution(graph: &DependencyGraph) -> ExecPlan {
    plan_execution_at(graph, None)
}

/// Build a plan, recording an optional project root for diagnostics.
pub fn plan_execution_at(graph: &DependencyGraph, root: Option<&Path>) -> ExecPlan {
    plan_execution_with_config(graph, root, None)
}

/// Build a plan enriched with on-disk discovery and config comparison.
pub fn plan_execution_with_config(
    graph: &DependencyGraph,
    root: Option<&Path>,
    cfg: Option<&ExecutionConfig>,
) -> ExecPlan {
    let mut entries = Vec::new();
    for node in graph.nodes.values() {
        if node.key.is_root() {
            continue;
        }
        if node.is_workspace && !node.key.as_str().starts_with("node_modules/") {
            continue;
        }
        let mut entry = classify_node(node);
        if let Some(root) = root {
            enrich_with_discovery(&mut entry, root, cfg);
        } else if let Some(cfg) = cfg {
            apply_config_policy(&mut entry, cfg);
        }
        entries.push(entry);
    }
    entries.sort_by(|a, b| a.package_key.cmp(&b.package_key));
    summarize_plan(entries, root)
}

fn summarize_plan(entries: Vec<ExecPlanEntry>, root: Option<&Path>) -> ExecPlan {
    let needs_execution_count = entries.iter().filter(|e| e.needs_execution).count();
    let would_execute_count = entries.iter().filter(|e| e.would_execute).count();
    let needs_review_count = entries
        .iter()
        .filter(|e| {
            matches!(
                e.policy,
                PolicyReviewStatus::NeedsReview
                    | PolicyReviewStatus::PartialCoverage
                    | PolicyReviewStatus::MetadataMissing
            )
        })
        .count();
    let needs_network_count = entries.iter().filter(|e| e.needs_network).count();
    let native_policy_gap_count = entries
        .iter()
        .filter(|e| {
            e.native_prebuilds.iter().any(|r| {
                !matches!(
                    r.status,
                    crate::prebuild_resolve::PrebuildResolveStatus::Configured
                        | crate::prebuild_resolve::PrebuildResolveStatus::Suggestable
                )
            })
        })
        .count();
    ExecPlan {
        root: root.map(|p| p.display().to_string()),
        entries,
        needs_execution_count,
        would_execute_count,
        needs_review_count,
        needs_network_count,
        native_policy_gap_count,
        executed: false,
    }
}

fn classify_node(node: &PackageNode) -> ExecPlanEntry {
    let name = node.name.clone();
    let package_key = node.key.as_str().to_owned();

    let name_l = name.as_deref().unwrap_or("").to_ascii_lowercase();
    if name_l.contains("electron-chromedriver") || name_l.ends_with("-installer-script") {
        return ExecPlanEntry {
            package_key,
            name,
            class: ExecNeedClass::UnsupportedUnsafe,
            needs_execution: false,
            would_execute: false,
            candidate_scripts: Vec::new(),
            discovered_scripts: Vec::new(),
            discovered_output_candidates: Vec::new(),
            allowed_outputs: Vec::new(),
            package_allowed: false,
            policy: PolicyReviewStatus::BlockedUnsafe,
            metadata_loaded: false,
            sandbox: ExecSandboxProfile::Offline,
            needs_network: false,
            prebuild: Vec::new(),
            native_prebuilds: Vec::new(),
            reason: "classified unsupported/unsafe — never auto-executed".into(),
        };
    }

    if node.likely_native || looks_native_name(&name_l) {
        let scripts = lifecycle_candidates(node);
        return ExecPlanEntry {
            package_key,
            name,
            class: ExecNeedClass::NativeBuild,
            needs_execution: true,
            would_execute: false,
            candidate_scripts: scripts,
            discovered_scripts: Vec::new(),
            discovered_output_candidates: Vec::new(),
            allowed_outputs: Vec::new(),
            package_allowed: false,
            policy: PolicyReviewStatus::MetadataMissing,
            metadata_loaded: false,
            sandbox: ExecSandboxProfile::PrebuildFetch,
            needs_network: false,
            prebuild: Vec::new(),
            native_prebuilds: Vec::new(),
            reason: "likely native addon — rebuild/prebuild may be required (docs/native.md)"
                .into(),
        };
    }

    if node.has_install_script {
        let scripts = lifecycle_candidates(node);
        return ExecPlanEntry {
            package_key,
            name,
            class: ExecNeedClass::GeneratedFiles,
            needs_execution: true,
            would_execute: false,
            candidate_scripts: scripts,
            discovered_scripts: Vec::new(),
            discovered_output_candidates: Vec::new(),
            allowed_outputs: Vec::new(),
            package_allowed: false,
            policy: PolicyReviewStatus::MetadataMissing,
            metadata_loaded: false,
            sandbox: ExecSandboxProfile::Offline,
            needs_network: false,
            prebuild: Vec::new(),
            native_prebuilds: Vec::new(),
            reason: "hasInstallScript — may generate runtime files".into(),
        };
    }

    ExecPlanEntry {
        package_key,
        name,
        class: ExecNeedClass::ExtractionOnly,
        needs_execution: false,
        would_execute: false,
        candidate_scripts: Vec::new(),
        discovered_scripts: Vec::new(),
        discovered_output_candidates: Vec::new(),
        allowed_outputs: Vec::new(),
        package_allowed: false,
        policy: PolicyReviewStatus::ExtractionOnly,
        metadata_loaded: false,
        sandbox: ExecSandboxProfile::Offline,
        needs_network: false,
        prebuild: Vec::new(),
        native_prebuilds: Vec::new(),
        reason: "extraction-only".into(),
    }
}

fn enrich_with_discovery(entry: &mut ExecPlanEntry, root: &Path, cfg: Option<&ExecutionConfig>) {
    let dir = resolve_package_dir_for_discovery(root, &entry.package_key, entry.name.as_deref());
    if let Some(dir) = dir {
        if let Ok(discovery) = discover_package_dir(&dir) {
            apply_discovery(entry, &discovery);
        }
        // Resolve native prebuild patterns even without project config (diagnostics only).
        let resolve_cfg = cfg.cloned().unwrap_or_default();
        if let Ok(report) = resolve_native_prebuilds_at(
            &dir,
            &resolve_cfg,
            &weave_core::HostPlatform::current(),
            &crate::exec::probe_node_identity()
                .map(|(a, _)| a)
                .unwrap_or_else(|_| "unknown".into()),
        ) {
            entry.native_prebuilds = report.requirements;
            if report.needs_manual_policy && !entry.needs_execution {
                entry.needs_execution = true;
                entry.class = ExecNeedClass::NativeBuild;
                entry.reason = format!(
                    "{}; native prebuild policy gap (see native_prebuilds)",
                    entry.reason
                );
            }
        }
    }
    if let Some(cfg) = cfg {
        apply_config_policy(entry, cfg);
    } else {
        // Without config, nothing is allowlisted — review required when needed.
        if entry.needs_execution && !matches!(entry.policy, PolicyReviewStatus::BlockedUnsafe) {
            entry.policy = if entry.metadata_loaded {
                PolicyReviewStatus::NeedsReview
            } else {
                PolicyReviewStatus::MetadataMissing
            };
        }
        entry.would_execute = false;
    }
}

fn apply_discovery(entry: &mut ExecPlanEntry, discovery: &PackageDiscovery) {
    entry.metadata_loaded = discovery.metadata_loaded;
    entry.discovered_scripts = discovery.discovered_scripts.clone();
    entry.discovered_output_candidates = discovery.output_candidates.clone();
    entry.class = discovery.class;
    entry.sandbox = discovery.sandbox;
    entry.needs_execution = discovery.needs_execution;
    entry.reason = discovery.reason.clone();
    if !discovery.discovered_scripts.is_empty() {
        entry.candidate_scripts = discovery
            .discovered_scripts
            .iter()
            .map(|s| s.name.clone())
            .collect();
    }
    if discovery.blocked_unsafe {
        entry.policy = PolicyReviewStatus::BlockedUnsafe;
        entry.needs_execution = false;
        entry.would_execute = false;
    }
}

fn apply_config_policy(entry: &mut ExecPlanEntry, cfg: &ExecutionConfig) {
    let name = entry.name.as_deref().unwrap_or("");
    entry.package_allowed = cfg.package_allowed(name);
    entry.allowed_outputs = cfg.outputs_for(name).to_vec();

    // Attach dry-run prebuild plan (never contacts the network).
    if let Ok(prebuilds) = plan_prebuild_for_package(cfg, name) {
        entry.prebuild = prebuilds;
        entry.needs_network = entry.prebuild.iter().any(|p| p.needs_network);
        if !entry.prebuild.is_empty() {
            entry.needs_execution = true;
            if cfg.allows_prebuild_network() {
                entry.sandbox = ExecSandboxProfile::PrebuildFetch;
            }
            if entry.reason == "extraction-only" {
                entry.reason = "configured allowlisted prebuild fetch".into();
                entry.class = ExecNeedClass::NativeBuild;
            }
        }
    }

    if matches!(entry.policy, PolicyReviewStatus::BlockedUnsafe)
        || matches!(entry.class, ExecNeedClass::UnsupportedUnsafe)
    {
        entry.would_execute = false;
        entry.policy = PolicyReviewStatus::BlockedUnsafe;
        entry.needs_network = false;
        return;
    }

    if !entry.needs_execution {
        entry.would_execute = false;
        entry.policy = PolicyReviewStatus::ExtractionOnly;
        return;
    }

    // Synthesize a discovery-like review when we have candidates; else use allowlist.
    if entry.metadata_loaded {
        let synth = PackageDiscovery {
            package_dir: PathBuf::new(),
            name: name.to_owned(),
            class: entry.class,
            sandbox: entry.sandbox,
            needs_execution: entry.needs_execution,
            discovered_scripts: entry.discovered_scripts.clone(),
            output_candidates: entry.discovered_output_candidates.clone(),
            metadata_loaded: true,
            reason: entry.reason.clone(),
            blocked_unsafe: false,
        };
        entry.policy = synth.review_against(cfg);
    } else if entry.package_allowed && !entry.allowed_outputs.is_empty() {
        entry.policy = PolicyReviewStatus::Allowed;
    } else if entry.package_allowed {
        entry.policy = PolicyReviewStatus::PartialCoverage;
    } else {
        entry.policy = PolicyReviewStatus::MetadataMissing;
    }

    // would_execute under --with-exec: allowlisted + has declared outputs + not blocked.
    entry.would_execute = entry.package_allowed
        && !entry.allowed_outputs.is_empty()
        && cfg.is_enabled()
        && !matches!(entry.policy, PolicyReviewStatus::BlockedUnsafe);

    // Even when enabled=false, surface allowlist readiness separately in policy;
    // would_execute stays false unless enabled (dual gate reminder in CLI).
    if entry.package_allowed
        && !entry.allowed_outputs.is_empty()
        && !cfg.is_enabled()
        && entry.needs_execution
    {
        // Policy may be Allowed (outputs cover) but dual gate still blocks run.
        entry.would_execute = false;
        if matches!(entry.policy, PolicyReviewStatus::Allowed) {
            entry.reason = format!(
                "{}; allowlisted but execution.enabled=false (dual gate)",
                entry.reason
            );
        }
    }
}

fn lifecycle_candidates(node: &PackageNode) -> Vec<String> {
    if node.has_install_script || node.likely_native {
        vec!["install".into(), "postinstall".into()]
    } else {
        Vec::new()
    }
}

fn looks_native_name(name: &str) -> bool {
    name.contains("sqlite3")
        || name.contains("bcrypt")
        || name.contains("fsevents")
        || name.contains("sharp")
        || name.contains("node-sass")
        || name.ends_with("-native")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use weave_core::{LockfileKind, PackageKey, PackageNode, PackageSource};

    fn graph_with(nodes: Vec<PackageNode>) -> DependencyGraph {
        let mut map = BTreeMap::new();
        map.insert(
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
        for n in nodes {
            map.insert(n.key.clone(), n);
        }
        DependencyGraph {
            lockfile_kind: LockfileKind::NpmPackageLock,
            lockfile_version: 3,
            root: PackageKey::root(),
            nodes: map,
            edges: Vec::new(),
        }
    }

    fn pkg(key: &str, name: &str, install: bool, native: bool) -> PackageNode {
        PackageNode {
            key: PackageKey::new(key),
            name: Some(name.into()),
            version: Some("1.0.0".into()),
            source: PackageSource::Registry {
                resolved: format!("https://example/{name}.tgz"),
            },
            integrity: None,
            dependencies: BTreeMap::new(),
            dev_dependencies: BTreeMap::new(),
            optional_dependencies: BTreeMap::new(),
            peer_dependencies: BTreeMap::new(),
            peer_dependencies_meta: BTreeMap::new(),
            has_install_script: install,
            optional: false,
            dev: false,
            peer: false,
            cpu: Vec::new(),
            os: Vec::new(),
            bundled_dependencies: Vec::new(),
            is_workspace: false,
            is_link: false,
            likely_native: native,
            bin: BTreeMap::new(),
        }
    }

    #[test]
    fn plan_marks_native_and_scripts_without_executing() {
        let g = graph_with(vec![
            pkg("node_modules/left-pad", "left-pad", false, false),
            pkg("node_modules/sqlite3", "sqlite3", true, true),
            pkg("node_modules/esbuild", "esbuild", true, false),
        ]);
        let plan = plan_execution(&g);
        assert!(!plan.executed);
        assert_eq!(plan.needs_execution_count, 2);
        assert_eq!(plan.would_execute_count, 0); // nothing allowlisted
        let sqlite = plan
            .entries
            .iter()
            .find(|e| e.name.as_deref() == Some("sqlite3"))
            .unwrap();
        assert_eq!(sqlite.class, ExecNeedClass::NativeBuild);
        assert!(sqlite.needs_execution);
        assert!(!sqlite.would_execute);
        assert_eq!(sqlite.sandbox, ExecSandboxProfile::PrebuildFetch);
        let plain = plan
            .entries
            .iter()
            .find(|e| e.name.as_deref() == Some("left-pad"))
            .unwrap();
        assert_eq!(plain.class, ExecNeedClass::ExtractionOnly);
        assert!(!plain.needs_execution);
    }

    #[test]
    fn plan_surfaces_native_prebuild_gaps_from_on_disk_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let nm = root.join("node_modules/demo-bcrypt-like");
        std::fs::create_dir_all(&nm).unwrap();
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/prebuild-resolve/node-pre-gyp-like/package.json");
        std::fs::copy(src, nm.join("package.json")).unwrap();

        let g = graph_with(vec![pkg(
            "node_modules/demo-bcrypt-like",
            "demo-bcrypt-like",
            true,
            true,
        )]);
        let plan = plan_execution_with_config(&g, Some(root), Some(&ExecutionConfig::default()));
        assert!(!plan.executed);
        assert!(plan.native_policy_gap_count >= 1);
        let entry = plan
            .entries
            .iter()
            .find(|e| e.name.as_deref() == Some("demo-bcrypt-like"))
            .unwrap();
        assert!(!entry.native_prebuilds.is_empty());
        assert!(entry.native_prebuilds.iter().any(|r| {
            matches!(
                r.status,
                crate::prebuild_resolve::PrebuildResolveStatus::NeedsIntegrity
                    | crate::prebuild_resolve::PrebuildResolveStatus::UnresolvedTokens
            )
        }));
        assert!(!entry.would_execute);
    }
}
