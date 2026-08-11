//! `weave doctor` diagnostics.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use weave_core::{Error, WEAVE_CONFIG, WEAVE_DIR};
use weave_git::GitRepository;
use weave_lockfile::{detect_lockfile, parse_lockfile};
use weave_store::{ArtifactId, ContentStore};

use crate::adoption::{assess_adoption, AdoptionAssessment, AdoptionVerdict};
use crate::config::ProjectConfig;
use crate::environment::EnvironmentStore;
use crate::exec_plan::plan_execution_with_config;
use crate::project::discover_project;

/// Severity of a diagnostic finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DoctorSeverity {
    /// Informational.
    Info,
    /// Warning — Weave may still work.
    Warn,
    /// Error — correctness or usability is blocked.
    Error,
}

/// One diagnostic finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorFinding {
    /// Severity.
    pub severity: DoctorSeverity,
    /// Stable check id.
    pub check: String,
    /// Human-readable message.
    pub message: String,
}

/// Aggregated doctor report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorReport {
    /// Project root inspected.
    pub root: PathBuf,
    /// Findings in stable order.
    pub findings: Vec<DoctorFinding>,
    /// Adoption readiness (when lockfile parses); never executes scripts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adoption: Option<AdoptionAssessment>,
}

impl DoctorReport {
    /// True when any error-severity finding exists.
    pub fn has_errors(&self) -> bool {
        self.findings
            .iter()
            .any(|f| f.severity == DoctorSeverity::Error)
    }
}

/// Run diagnostics for the project containing `start`.
pub fn doctor_project(start: &Path) -> weave_core::Result<DoctorReport> {
    let mut findings = Vec::new();
    let mut adoption: Option<AdoptionAssessment> = None;

    let git = match GitRepository::discover(start) {
        Ok(repo) => {
            findings.push(finding(
                DoctorSeverity::Info,
                "git",
                format!(
                    "repository {} (branch {})",
                    repo.root.display(),
                    repo.branch.as_deref().unwrap_or("detached")
                ),
            ));
            Some(repo)
        }
        Err(err) => {
            findings.push(finding(DoctorSeverity::Error, "git", err.to_string()));
            None
        }
    };

    let root = git
        .as_ref()
        .map(|r| r.root.clone())
        .unwrap_or_else(|| start.to_path_buf());

    match discover_project(&root) {
        Ok(discovery) => {
            if discovery.layout.package_json.exists() {
                findings.push(finding(
                    DoctorSeverity::Info,
                    "package-json",
                    "package.json present",
                ));
            }
            let loaded_cfg = if discovery.layout.weave_initialized {
                ProjectConfig::load(&discovery.layout.root).ok()
            } else {
                None
            };
            match &discovery.layout.lockfile {
                Some(path) => match parse_lockfile(path) {
                    Ok(graph) => {
                        findings.push(finding(
                            DoctorSeverity::Info,
                            "lockfile",
                            format!(
                                "{} ok ({} packages, id {}…)",
                                path.file_name()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("lockfile"),
                                graph.package_count(),
                                &graph.identity().as_str()[..12]
                            ),
                        ));
                        let scripted = graph.packages_with_install_scripts();
                        if !scripted.is_empty() {
                            findings.push(finding(
                                DoctorSeverity::Warn,
                                "lifecycle-scripts",
                                format!(
                                    "{} package(s) declare install scripts; plain `weave switch` \
                                     does not execute them (docs/lifecycle.md). Run `weave exec plan` \
                                     to see whether opt-in sandboxed execution could complete them — \
                                     scripts never get open network.",
                                    scripted.len()
                                ),
                            ));
                        }
                        let peers = graph.audit_peers();
                        let missing_req = peers
                            .iter()
                            .filter(|f| {
                                matches!(f.status, weave_core::PeerAuditStatus::MissingRequired)
                            })
                            .count();
                        let missing_opt = peers
                            .iter()
                            .filter(|f| {
                                matches!(f.status, weave_core::PeerAuditStatus::MissingOptional)
                            })
                            .count();
                        if missing_req > 0 {
                            findings.push(finding(
                                DoctorSeverity::Error,
                                "peer-dependencies",
                                format!(
                                    "{missing_req} required peerDependencies unsatisfied in lockfile. \
                                     Weave does not auto-install peers — add them with npm and re-lock \
                                     before `weave switch` can produce a correct environment."
                                ),
                            ));
                        } else if !peers.is_empty() {
                            findings.push(finding(
                                DoctorSeverity::Info,
                                "peer-dependencies",
                                format!(
                                    "{} peer edge(s) audited ({} optional missing)",
                                    peers.len(),
                                    missing_opt
                                ),
                            ));
                        }
                        let host = weave_core::HostPlatform::current();
                        let mut skip_opt = 0usize;
                        let mut reject = 0usize;
                        for node in graph.nodes.values() {
                            match weave_core::platform_fit(node, &host) {
                                weave_core::PlatformFit::SkipOptional => skip_opt += 1,
                                weave_core::PlatformFit::RejectRequired => reject += 1,
                                weave_core::PlatformFit::Compatible => {}
                            }
                        }
                        if skip_opt > 0 {
                            findings.push(finding(
                                DoctorSeverity::Info,
                                "optional-platform",
                                format!(
                                    "{skip_opt} optional package(s) incompatible with {}/{} — will be skipped",
                                    host.npm_os(),
                                    host.npm_cpu()
                                ),
                            ));
                        }
                        if reject > 0 {
                            findings.push(finding(
                                DoctorSeverity::Error,
                                "platform-required",
                                format!(
                                    "{reject} required package(s) incompatible with {}/{} — replace \
                                     those deps or run on a matching platform before adopting Weave",
                                    host.npm_os(),
                                    host.npm_cpu()
                                ),
                            ));
                        }
                        let native = graph.nodes.values().filter(|n| n.likely_native).count();
                        if native > 0 {
                            findings.push(finding(
                                DoctorSeverity::Warn,
                                "native-addons",
                                format!(
                                    "{native} package(s) look native/addon-related; Weave copies \
                                     prebuilt artifacts only and does not rebuild. If binaries are \
                                     missing after switch, declare reviewed prebuild.fetches (with SRI) \
                                     or rebuild outside Weave — see docs/native.md and docs/adoption.md"
                                ),
                            ));
                            inspect_native_tree(&discovery.layout.root, &mut findings);
                        }
                        let exec_cfg = loaded_cfg.as_ref().map(|c| &c.execution);
                        let plan = plan_execution_with_config(
                            &graph,
                            Some(&discovery.layout.root),
                            exec_cfg,
                        );
                        let assessment = assess_adoption(&graph, &plan, exec_cfg);
                        push_adoption_findings(&assessment, &mut findings);
                        adoption = Some(assessment);
                    }
                    Err(err) => {
                        findings.push(finding(DoctorSeverity::Error, "lockfile", err.to_string()))
                    }
                },
                None => findings.push(finding(
                    DoctorSeverity::Error,
                    "lockfile",
                    "package-lock.json missing — Weave requires an npm lockfile (run npm install \
                     to generate one, then weave init)",
                )),
            }

            if discovery.layout.weave_initialized {
                findings.push(finding(
                    DoctorSeverity::Info,
                    "weave-init",
                    ".weave/config.toml present",
                ));
                match loaded_cfg {
                    Some(config) => {
                        let store_path = PathBuf::from(&config.store_path);
                        if store_path.join("sha256").is_dir() {
                            findings.push(finding(
                                DoctorSeverity::Info,
                                "store",
                                format!("object store ok at {}", store_path.display()),
                            ));
                            verify_environment_artifacts(
                                &discovery.layout.root,
                                &store_path,
                                &mut findings,
                            )?;
                        } else {
                            findings.push(finding(
                                DoctorSeverity::Warn,
                                "store",
                                format!(
                                    "store path {} missing sha256/ — run weave init or check WEAVE_HOME",
                                    store_path.display()
                                ),
                            ));
                        }
                    }
                    None => {
                        // load failed earlier or race — try again for message
                        if let Err(err) = ProjectConfig::load(&discovery.layout.root) {
                            findings.push(finding(
                                DoctorSeverity::Error,
                                "weave-config",
                                err.to_string(),
                            ));
                        }
                    }
                }

                let env_store = EnvironmentStore::open(&discovery.layout.root);
                match env_store.active_id() {
                    Ok(Some(id)) => match env_store.get(&id) {
                        Ok(_) => findings.push(finding(
                            DoctorSeverity::Info,
                            "active-env",
                            format!("active environment {id}"),
                        )),
                        Err(_) => findings.push(finding(
                            DoctorSeverity::Error,
                            "active-env",
                            format!("active pointer {id} has no environment record"),
                        )),
                    },
                    Ok(None) => findings.push(finding(
                        DoctorSeverity::Info,
                        "active-env",
                        "no active environment pointer — run `weave switch` after init",
                    )),
                    Err(err) => {
                        findings.push(finding(DoctorSeverity::Warn, "active-env", err.to_string()))
                    }
                }
            } else {
                findings.push(finding(
                    DoctorSeverity::Warn,
                    "weave-init",
                    "Weave not initialized — run `weave init` then `weave switch` \
                     (docs/adoption.md)",
                ));
                let _ = detect_lockfile(&discovery.layout.root);
            }
        }
        Err(err) => {
            // discover may fail before weave init; still report.
            if !matches!(err, Error::NotAGitRepository { .. }) {
                findings.push(finding(DoctorSeverity::Error, "project", err.to_string()));
            }
        }
    }

    // Orphan candidate / backup leftovers.
    let candidate = root.join(WEAVE_DIR).join(weave_core::WEAVE_CANDIDATE_DIR);
    if candidate.exists() {
        findings.push(finding(
            DoctorSeverity::Warn,
            "candidate",
            format!(
                "leftover candidate at {} (incomplete switch?) — safe to remove after \
                 confirming active node_modules is intact",
                candidate.display()
            ),
        ));
    }
    let backup = root
        .join(WEAVE_DIR)
        .join(weave_core::WEAVE_BACKUP_NODE_MODULES);
    if backup.exists() {
        findings.push(finding(
            DoctorSeverity::Warn,
            "backup-node-modules",
            format!(
                "leftover backup at {} (interrupted activation?) — inspect before deleting",
                backup.display()
            ),
        ));
    }

    let _ = fs::metadata(root.join(WEAVE_DIR).join(WEAVE_CONFIG));

    Ok(DoctorReport {
        root,
        findings,
        adoption,
    })
}

fn push_adoption_findings(assessment: &AdoptionAssessment, findings: &mut Vec<DoctorFinding>) {
    let (severity, check) = match assessment.verdict {
        AdoptionVerdict::ExtractionReady => (DoctorSeverity::Info, "adoption"),
        AdoptionVerdict::PartialNeedsPolicy => (DoctorSeverity::Warn, "adoption"),
        AdoptionVerdict::Blocked => (DoctorSeverity::Error, "adoption"),
    };
    findings.push(finding(severity, check, assessment.summary.clone()));
    for action in assessment.next_actions.iter().take(4) {
        findings.push(finding(
            DoctorSeverity::Info,
            "adoption-next",
            format!("{} — {}", action.step, action.why),
        ));
    }
    // Keep a stable exec-plan check for older consumers.
    if assessment.needs_execution_count > 0 {
        findings.push(finding(
            DoctorSeverity::Info,
            "exec-plan",
            format!(
                "ADR-0018 dry-run: {} package(s) need opt-in sandboxed execution for \
                 completeness; {} native policy gap(s) (executed=false; weave exec plan / suggest)",
                assessment.needs_execution_count, assessment.native_policy_gap_count
            ),
        ));
    } else {
        findings.push(finding(
            DoctorSeverity::Info,
            "exec-plan",
            "ADR-0018 dry-run: no packages classified as needing execution — \
             no [execution] configuration required",
        ));
    }
}

fn verify_environment_artifacts(
    project_root: &Path,
    store_path: &Path,
    findings: &mut Vec<DoctorFinding>,
) -> weave_core::Result<()> {
    let store = ContentStore::open(store_path)?;
    let envs = EnvironmentStore::open(project_root).list()?;
    let mut missing = 0usize;
    let mut checked = 0usize;
    for env in &envs {
        for id_str in env.artifacts.values() {
            checked += 1;
            match ArtifactId::parse(id_str) {
                Ok(id) => {
                    if !store.contains(&id) {
                        missing += 1;
                    } else if store.verify(&id).is_err() {
                        findings.push(finding(
                            DoctorSeverity::Error,
                            "artifact-corrupt",
                            format!("corrupt artifact {id}"),
                        ));
                    }
                }
                Err(_) => {
                    findings.push(finding(
                        DoctorSeverity::Warn,
                        "artifact-id",
                        format!("invalid artifact id in env {}: {id_str}", env.id),
                    ));
                }
            }
        }
    }
    if checked == 0 {
        findings.push(finding(
            DoctorSeverity::Info,
            "artifacts",
            "no environment artifacts recorded yet",
        ));
    } else if missing == 0 {
        findings.push(finding(
            DoctorSeverity::Info,
            "artifacts",
            format!("all {checked} recorded artifacts present in store"),
        ));
    } else {
        findings.push(finding(
            DoctorSeverity::Error,
            "artifacts",
            format!("{missing}/{checked} recorded artifacts missing from store"),
        ));
    }
    Ok(())
}

fn finding(severity: DoctorSeverity, check: &str, message: impl Into<String>) -> DoctorFinding {
    DoctorFinding {
        severity,
        check: check.to_owned(),
        message: message.into(),
    }
}

/// Inspect activated `node_modules` for native packages missing `.node` binaries.
fn inspect_native_tree(root: &Path, findings: &mut Vec<DoctorFinding>) {
    let nm = root.join("node_modules");
    if !nm.is_dir() {
        findings.push(finding(
            DoctorSeverity::Info,
            "native-rebuild",
            "no node_modules yet — native rebuild needs cannot be verified until after switch",
        ));
        return;
    }
    let mut missing_binary = 0usize;
    let mut checked = 0usize;
    let _ = walk_native_hint(&nm, &nm, &mut checked, &mut missing_binary);
    if checked == 0 {
        return;
    }
    if missing_binary > 0 {
        findings.push(finding(
            DoctorSeverity::Warn,
            "native-rebuild",
            format!(
                "{missing_binary}/{checked} native-looking package(s) lack a .node binary after \
                 materialization — this is expected when install scripts were not run. Rebuild \
                 outside Weave, or add reviewed execution.prebuild.fetches with SRI \
                 (docs/native.md / docs/adoption.md). Weave will not invent integrity or open \
                 script networking."
            ),
        ));
    } else {
        findings.push(finding(
            DoctorSeverity::Info,
            "native-rebuild",
            format!("{checked} native-looking package(s) include prebuilt .node artifacts"),
        ));
    }
}

fn walk_native_hint(
    nm_root: &Path,
    dir: &Path,
    checked: &mut usize,
    missing: &mut usize,
) -> weave_core::Result<()> {
    let pkg_json = dir.join("package.json");
    if pkg_json.is_file() && dir != nm_root {
        let rel = dir.strip_prefix(nm_root).unwrap_or(dir);
        let name = rel.to_string_lossy();
        // Only inspect package roots (direct children of node_modules or @scope).
        let is_pkg_root = {
            let comps: Vec<_> = rel.components().collect();
            matches!(comps.len(), 1 | 2)
                && comps
                    .first()
                    .map(|c| {
                        let s = c.as_os_str().to_string_lossy();
                        s.starts_with('@') || comps.len() == 1
                    })
                    .unwrap_or(false)
        };
        if is_pkg_root {
            let bytes = fs::read(&pkg_json).unwrap_or_default();
            let text = String::from_utf8_lossy(&bytes);
            let looks = text.contains("node-gyp")
                || text.contains("node-addon-api")
                || text.contains("\"binary\"")
                || dir.join("binding.gyp").exists()
                || name.contains("sqlite3")
                || name.contains("bcrypt")
                || name.contains("fsevents")
                || name.contains("sharp");
            if looks {
                *checked += 1;
                if !dir_has_node_binary(dir) {
                    *missing += 1;
                }
            }
        }
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir()
                && path.file_name().and_then(|s| s.to_str()) != Some(".bin")
                && path.file_name().and_then(|s| s.to_str()) != Some(".weave")
            {
                // Limit depth: only walk top-level and scoped packages.
                let rel = path.strip_prefix(nm_root).unwrap_or(&path);
                if rel.components().count() <= 3 {
                    let _ = walk_native_hint(nm_root, &path, checked, missing);
                }
            }
        }
    }
    Ok(())
}

fn dir_has_node_binary(dir: &Path) -> bool {
    fn walk(path: &Path, depth: usize) -> bool {
        if depth > 4 {
            return false;
        }
        let Ok(entries) = fs::read_dir(path) else {
            return false;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) == Some("node") {
                return true;
            }
            if p.is_dir() && walk(&p, depth + 1) {
                return true;
            }
        }
        false
    }
    walk(dir, 0)
}
