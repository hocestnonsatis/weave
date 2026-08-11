//! Real-world adoption readiness (ADR-0018 Phase 12).
//!
//! Turns dry-run exec plans and lockfile audits into a short verdict plus
//! **actionable next steps** — what the developer must do and why — without
//! inventing SRI, URLs, outputs, or permissions, and without weakening the
//! security model.

use serde::{Deserialize, Serialize};
use weave_core::{DependencyGraph, PeerAuditStatus};

use crate::config::ExecutionConfig;
use crate::exec_plan::{ExecNeedClass, ExecPlan};
use crate::prebuild_resolve::PrebuildResolveStatus;

/// High-level readiness of a project for Weave.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdoptionVerdict {
    /// Ordinary extraction-only tree: plain `weave switch` is enough.
    ExtractionReady,
    /// Switch can materialize tarballs, but some packages stay incomplete
    /// until reviewed execution/prebuild policy is added.
    PartialNeedsPolicy,
    /// Hard blockers (peers, platform, unsafe packages) prevent a correct env.
    Blocked,
}

/// One concrete next action for the developer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdoptionAction {
    /// Short command or config step.
    pub step: String,
    /// Why this step is required.
    pub why: String,
}

/// Per-package gap that blocks completeness (not an auto-fix).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdoptionPackageGap {
    /// Package name when known.
    pub package: String,
    /// Classification label.
    pub class: String,
    /// What Weave detected.
    pub issue: String,
    /// What the human must do (never auto-done).
    pub user_must: String,
}

/// Project adoption assessment for doctor / plan / suggest UX.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdoptionAssessment {
    /// Overall verdict.
    pub verdict: AdoptionVerdict,
    /// One-line summary.
    pub summary: String,
    /// Ordered next actions.
    pub next_actions: Vec<AdoptionAction>,
    /// Packages needing attention.
    pub gaps: Vec<AdoptionPackageGap>,
    /// Packages that are extraction-only.
    pub extraction_only_count: usize,
    /// Packages classified as needing opt-in execution.
    pub needs_execution_count: usize,
    /// Packages blocked as unsupported/unsafe.
    pub blocked_count: usize,
    /// Packages with unresolved native prebuild policy.
    pub native_policy_gap_count: usize,
    /// True when any execution configuration is required for completeness.
    pub execution_config_required: bool,
    /// Reminder: assessment never executed scripts or contacted the network.
    pub executed: bool,
}

/// Assess adoption readiness from a dry-run plan + graph peer audit.
///
/// Does not invent SRI/URLs/outputs and never enables execution.
pub fn assess_adoption(
    graph: &DependencyGraph,
    plan: &ExecPlan,
    cfg: Option<&ExecutionConfig>,
) -> AdoptionAssessment {
    let peers = graph.audit_peers();
    let missing_required_peers = peers
        .iter()
        .filter(|f| matches!(f.status, PeerAuditStatus::MissingRequired))
        .count();
    let host = weave_core::HostPlatform::current();
    let reject_platform = graph
        .nodes
        .values()
        .filter(|n| {
            matches!(
                weave_core::platform_fit(n, &host),
                weave_core::PlatformFit::RejectRequired
            )
        })
        .count();

    let extraction_only_count = plan
        .entries
        .iter()
        .filter(|e| matches!(e.class, ExecNeedClass::ExtractionOnly) && !e.needs_execution)
        .count();
    let needs_execution_count = plan.needs_execution_count;
    let blocked_count = plan
        .entries
        .iter()
        .filter(|e| matches!(e.class, ExecNeedClass::UnsupportedUnsafe))
        .count();
    let native_policy_gap_count = plan.native_policy_gap_count;

    let mut gaps = Vec::new();
    for e in &plan.entries {
        if matches!(e.class, ExecNeedClass::UnsupportedUnsafe) {
            gaps.push(AdoptionPackageGap {
                package: e.name.clone().unwrap_or_else(|| e.package_key.clone()),
                class: "unsupported-unsafe".into(),
                issue: e.reason.clone(),
                user_must: "Do not allowlist this package. Replace it, vendor a safe build, \
                            or keep it out of Weave-managed trees — Weave will not grant \
                            open network or execute unsafe install scripts."
                    .into(),
            });
            continue;
        }
        if !e.needs_execution {
            continue;
        }
        let name = e.name.clone().unwrap_or_else(|| e.package_key.clone());
        if let Some(np) = e.native_prebuilds.iter().find(|r| {
            !matches!(
                r.status,
                PrebuildResolveStatus::Configured | PrebuildResolveStatus::Suggestable
            )
        }) {
            let (issue, user_must) = match np.status {
                PrebuildResolveStatus::NeedsIntegrity => (
                    format!(
                        "native download pattern {:?}; URL/output known but integrity missing",
                        np.pattern
                    ),
                    "Verify the artifact yourself, then add an explicit \
                     [[execution.prebuild.fetches]] entry with HTTPS url, SRI integrity, \
                     and sealed relative output. Weave will not invent SRI. See docs/native.md."
                        .to_owned(),
                ),
                PrebuildResolveStatus::UnresolvedTokens => (
                    format!(
                        "native URL template still has tokens {:?}: {}",
                        np.unresolved_tokens, np.reason
                    ),
                    "Resolve or hardcode the concrete URL/output in \
                     execution.prebuild.fetches after review — do not rely on install scripts \
                     (they get no network)."
                        .to_owned(),
                ),
                PrebuildResolveStatus::Opaque => (
                    np.reason.clone(),
                    "Declare url + integrity + output manually in execution.prebuild.fetches \
                     if you need the binary under Weave; otherwise rebuild outside Weave and \
                     treat the result as external. Weave will not guess dynamic download URLs."
                        .to_owned(),
                ),
                PrebuildResolveStatus::BlockedUnsafe => (
                    np.reason.clone(),
                    "Refuse this download path (HTTP/unsafe). Use HTTPS + allowlisted host + SRI \
                     only — never enable open networking for lifecycle scripts."
                        .to_owned(),
                ),
                PrebuildResolveStatus::Configured | PrebuildResolveStatus::Suggestable => {
                    unreachable!("filtered above")
                }
            };
            gaps.push(AdoptionPackageGap {
                package: name.clone(),
                class: format!("{:?}", e.class),
                issue,
                user_must,
            });
            continue;
        }
        if e.native_prebuilds
            .iter()
            .any(|r| r.status == PrebuildResolveStatus::Suggestable)
        {
            gaps.push(AdoptionPackageGap {
                package: name.clone(),
                class: format!("{:?}", e.class),
                issue: "concrete HTTPS + output + SRI found in metadata (reviewable draft)".into(),
                user_must: "Run `weave exec suggest`, review the TOML draft, merge only after \
                            verifying the artifact, set profile=prebuild-fetch if needed, then \
                            enable execution + `weave switch --with-exec`. Never auto-approve."
                    .into(),
            });
            continue;
        }
        gaps.push(AdoptionPackageGap {
            package: name,
            class: format!("{:?}", e.class),
            issue: e.reason.clone(),
            user_must: if e.discovered_output_candidates.iter().any(|c| c.safe) {
                "Run `weave exec suggest`, review allow_packages + declared_outputs, \
                 enable execution only after review, then `weave switch --with-exec`. \
                 Plain switch stays execution-free."
                    .into()
            } else {
                "Package needs generated files but Weave could not establish exact safe \
                 output paths from metadata. Declare exact relative outputs manually in \
                 .weave/config.toml after inspection — Weave will not invent paths."
                    .into()
            },
        });
    }

    let hard_block = missing_required_peers > 0 || reject_platform > 0 || blocked_count > 0;
    let execution_config_required = needs_execution_count > 0;

    let (verdict, summary) = if hard_block {
        let mut parts = Vec::new();
        if missing_required_peers > 0 {
            parts.push(format!(
                "{missing_required_peers} unsatisfied required peer(s)"
            ));
        }
        if reject_platform > 0 {
            parts.push(format!(
                "{reject_platform} required package(s) incompatible with host platform"
            ));
        }
        if blocked_count > 0 {
            parts.push(format!(
                "{blocked_count} unsupported/unsafe package(s) that Weave will not execute"
            ));
        }
        (
            AdoptionVerdict::Blocked,
            format!(
                "Adoption blocked: {}. Fix lockfile/deps before relying on Weave for a complete env.",
                parts.join("; ")
            ),
        )
    } else if !execution_config_required {
        (
            AdoptionVerdict::ExtractionReady,
            format!(
                "Extraction-ready: {extraction_only_count} package(s) need only tarball \
                 materialization. No execution configuration required — plain `weave switch` \
                 is the complete path."
            ),
        )
    } else {
        (
            AdoptionVerdict::PartialNeedsPolicy,
            format!(
                "Partial: plain `weave switch` materializes tarballs, but \
                 {needs_execution_count} package(s) need reviewed opt-in policy for completeness \
                 ({} native policy gap(s)). Environment may run without them if unused at runtime.",
                native_policy_gap_count
            ),
        )
    };

    let mut next_actions = Vec::new();
    match verdict {
        AdoptionVerdict::ExtractionReady => {
            next_actions.push(AdoptionAction {
                step: "weave init && weave switch".into(),
                why: "Initialize Weave metadata and materialize node_modules from the lockfile \
                      (offline after artifacts are in the CAS)."
                    .into(),
            });
            next_actions.push(AdoptionAction {
                step: "weave doctor".into(),
                why: "Confirm store, peers, and that no unexpected lifecycle/native gaps appear."
                    .into(),
            });
            next_actions.push(AdoptionAction {
                step: "node <your-entrypoint>".into(),
                why: "Run the app against the activated tree — no [execution] block needed.".into(),
            });
        }
        AdoptionVerdict::PartialNeedsPolicy => {
            next_actions.push(AdoptionAction {
                step: "weave switch".into(),
                why: "Materialize extraction-only packages now. Plain switch never executes \
                      and never opens network for install scripts."
                    .into(),
            });
            next_actions.push(AdoptionAction {
                step: "weave exec plan".into(),
                why: "See which packages are incomplete and why (native gaps, missing outputs)."
                    .into(),
            });
            next_actions.push(AdoptionAction {
                step: "weave exec suggest".into(),
                why: "Emit reviewable allowlist/prebuild drafts only when safe metadata exists. \
                      Suggestions never set enabled=true."
                    .into(),
            });
            if native_policy_gap_count > 0 {
                next_actions.push(AdoptionAction {
                    step: "add execution.prebuild.fetches (manual SRI)".into(),
                    why: "Packages lacking static integrity cannot be auto-suggested. Verify \
                          artifacts offline, then declare HTTPS url + SRI + output yourself. \
                          Docs: docs/native.md / docs/adoption.md."
                        .into(),
                });
            }
            next_actions.push(AdoptionAction {
                step: "enable + weave switch --with-exec".into(),
                why: "Only after reviewing config: set execution.enabled=true (dual gate). \
                      Env vars cannot enable execution."
                    .into(),
            });
        }
        AdoptionVerdict::Blocked => {
            if missing_required_peers > 0 {
                next_actions.push(AdoptionAction {
                    step: "fix package-lock.json peers".into(),
                    why: "Weave does not auto-install missing required peerDependencies — \
                          add them to the lockfile with npm/yarn and re-lock."
                        .into(),
                });
            }
            if reject_platform > 0 {
                next_actions.push(AdoptionAction {
                    step: "replace platform-incompatible required deps".into(),
                    why: format!(
                        "Required packages reject host {}/{}.",
                        host.npm_os(),
                        host.npm_cpu()
                    ),
                });
            }
            if blocked_count > 0 {
                next_actions.push(AdoptionAction {
                    step: "remove or replace unsafe install-script packages".into(),
                    why: "Weave fails closed on packages classified unsupported/unsafe rather \
                          than granting open network to lifecycle scripts."
                        .into(),
                });
            }
            next_actions.push(AdoptionAction {
                step: "weave doctor && weave exec plan".into(),
                why: "Re-check after lockfile/dependency fixes.".into(),
            });
        }
    }

    if let Some(cfg) = cfg {
        if cfg.is_enabled() && matches!(verdict, AdoptionVerdict::PartialNeedsPolicy) {
            next_actions.insert(
                0,
                AdoptionAction {
                    step: "weave switch --with-exec".into(),
                    why:
                        "execution.enabled is already true — dual gate still requires --with-exec \
                          for sandboxed apply."
                            .into(),
                },
            );
        }
    }

    AdoptionAssessment {
        verdict,
        summary,
        next_actions,
        gaps,
        extraction_only_count,
        needs_execution_count,
        blocked_count,
        native_policy_gap_count,
        execution_config_required,
        executed: false,
    }
}

/// Render a human-readable adoption block for CLI / doctor.
pub fn render_adoption_text(a: &AdoptionAssessment) -> String {
    let mut out = String::new();
    out.push_str(&format!("Adoption: {:?} — {}\n", a.verdict, a.summary));
    if !a.gaps.is_empty() {
        out.push_str("Gaps:\n");
        for g in &a.gaps {
            out.push_str(&format!(
                "  - {} [{}]: {}\n      → {}\n",
                g.package, g.class, g.issue, g.user_must
            ));
        }
    }
    out.push_str("Next:\n");
    for (i, step) in a.next_actions.iter().enumerate() {
        out.push_str(&format!("  {}. {} — {}\n", i + 1, step.step, step.why));
    }
    out.push_str(
        "Note: assessment never executed scripts or contacted the network; \
         Weave never invents SRI, URLs, outputs, or permissions.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec_plan::{plan_execution_with_config, ExecNeedClass, ExecSandboxProfile};
    use crate::prebuild_resolve::{NativePrebuildRequirement, PrebuildPatternKind};
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
    fn extraction_only_is_ready_without_exec_config() {
        let g = graph_with(vec![pkg("node_modules/ms", "ms", false, false)]);
        let plan = plan_execution_with_config(&g, None, None);
        let a = assess_adoption(&g, &plan, None);
        assert_eq!(a.verdict, AdoptionVerdict::ExtractionReady);
        assert!(!a.execution_config_required);
        assert!(a.summary.contains("No execution configuration"));
        let text = render_adoption_text(&a);
        assert!(text.contains("weave switch"));
    }

    #[test]
    fn native_gap_is_partial_with_manual_sri_guidance() {
        let g = graph_with(vec![pkg(
            "node_modules/demo-bcrypt-like",
            "demo-bcrypt-like",
            true,
            true,
        )]);
        let mut plan = plan_execution_with_config(&g, None, None);
        if let Some(e) = plan.entries.iter_mut().next() {
            e.needs_execution = true;
            e.class = ExecNeedClass::NativeBuild;
            e.sandbox = ExecSandboxProfile::PrebuildFetch;
            e.native_prebuilds = vec![NativePrebuildRequirement {
                package: "demo-bcrypt-like".into(),
                pattern: PrebuildPatternKind::NodePreGypBinary,
                status: PrebuildResolveStatus::NeedsIntegrity,
                url: Some("https://cdn.example.com/x.tar.gz".into()),
                host: Some("cdn.example.com".into()),
                output: Some("lib/binding/x.node".into()),
                integrity: None,
                node_abi: Some("137".into()),
                os: Some("linux".into()),
                cpu: Some("x64".into()),
                unresolved_tokens: Vec::new(),
                reason: "integrity missing".into(),
            }];
        }
        plan.needs_execution_count = 1;
        plan.native_policy_gap_count = 1;
        let a = assess_adoption(&g, &plan, None);
        assert_eq!(a.verdict, AdoptionVerdict::PartialNeedsPolicy);
        assert!(a.execution_config_required);
        assert!(a.gaps.iter().any(|g| g.user_must.contains("SRI")));
        let text = render_adoption_text(&a);
        assert!(text.contains("weave exec plan"));
        assert!(text.contains("never invents"));
    }
}
