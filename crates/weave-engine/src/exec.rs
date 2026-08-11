//! Opt-in sandboxed lifecycle execution (ADR-0018 Phase 7–8).
//!
//! Offline Bubblewrap runs against an ephemeral work tree (never the live
//! `node_modules`), sealing only declared outputs into the CAS. Phase 8
//! applies sealed outputs onto the isolated candidate before activation when
//! both config opt-in and `--with-exec` are present.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use weave_core::{Error, HostPlatform};
use weave_fs::{extract_npm_tarball, pack_directory_as_npm_tarball};
use weave_lockfile::parse_project_lockfile;
use weave_store::{hash_bytes, ArtifactId, ContentStore};

use crate::config::{ExecutionConfig, ProjectConfig};
use crate::exec_plan::{plan_execution_with_config, ExecPlan, ExecPlanEntry};
use crate::project::discover_project;

/// Result of sealing declared execution outputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecSealReport {
    /// Content-addressed id of the sealed output tarball.
    pub output_artifact_id: ArtifactId,
    /// Relative paths sealed.
    pub sealed_paths: Vec<String>,
    /// Cache identity key for this run.
    pub cache_key: String,
}

/// Report from one sandboxed package execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecRunReport {
    /// Package name executed.
    pub package: String,
    /// Ephemeral work root used (under WEAVE_HOME/exec).
    pub work_root: PathBuf,
    /// Seal report when successful.
    pub seal: ExecSealReport,
    /// Node ABI (`process.versions.modules`) recorded for identity.
    pub node_abi: String,
    /// Host OS (npm token).
    pub platform_os: String,
    /// Host CPU (npm token).
    pub platform_cpu: String,
}

/// Resolve bwrap binary (overridable for tests via `WEAVE_BWRAP_PATH`).
pub fn bwrap_bin() -> PathBuf {
    std::env::var_os("WEAVE_BWRAP_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("bwrap"))
}

/// True when the configured bwrap binary appears runnable.
pub fn bwrap_available() -> bool {
    Command::new(bwrap_bin())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Refuse execution unless version-controlled config enables it.
pub fn require_execution_enabled(cfg: &ExecutionConfig) -> weave_core::Result<()> {
    // Ambient env must never enable execution (ADR-0018 / Phase 7).
    let _ = std::env::var_os("WEAVE_EXEC");
    let _ = std::env::var_os("WEAVE_EXEC_TESTS");
    if !cfg.is_enabled() {
        return Err(Error::InvalidState {
            path: PathBuf::from(".weave/config.toml"),
            reason: "execution is disabled: set [execution] enabled = true in \
                     version-controlled .weave/config.toml (env vars cannot enable execution)"
                .into(),
        });
    }
    cfg.validate().map_err(|reason| Error::InvalidState {
        path: PathBuf::from(".weave/config.toml"),
        reason,
    })?;
    Ok(())
}

/// Fail closed when Bubblewrap is unavailable.
pub fn require_sandbox() -> weave_core::Result<()> {
    if bwrap_available() {
        return Ok(());
    }
    Err(Error::InvalidState {
        path: bwrap_bin(),
        reason: "sandbox unavailable: bubblewrap (bwrap) is required for execution; \
                 Weave fails closed and will not run unsandboxed (ADR-0018)"
            .into(),
    })
}

/// Build a dry-run execution plan for the project (filtered by config allowlists).
pub fn exec_plan_for_project(start: &Path) -> weave_core::Result<(ExecPlan, ProjectConfig)> {
    let discovery = discover_project(start)?;
    let root = discovery.layout.root;
    let cfg = ProjectConfig::load(&root)?;
    let graph = parse_project_lockfile(&root)?;
    let plan = plan_execution_with_config(&graph, Some(&root), Some(&cfg.execution));
    Ok((plan, cfg))
}

/// Plan + adoption readiness for CLI / tooling (never executes scripts).
pub fn exec_plan_with_adoption(
    start: &Path,
) -> weave_core::Result<(ExecPlan, ProjectConfig, crate::adoption::AdoptionAssessment)> {
    let discovery = discover_project(start)?;
    let root = discovery.layout.root;
    let cfg = ProjectConfig::load(&root)?;
    let graph = parse_project_lockfile(&root)?;
    let plan = plan_execution_with_config(&graph, Some(&root), Some(&cfg.execution));
    let adoption = crate::adoption::assess_adoption(&graph, &plan, Some(&cfg.execution));
    Ok((plan, cfg, adoption))
}

/// Discover package metadata for all graph nodes that have on-disk trees.
pub fn discover_policies_for_project(
    start: &Path,
) -> weave_core::Result<(
    Vec<crate::exec_discover::PackageDiscovery>,
    ProjectConfig,
    ExecPlan,
)> {
    let discovery = discover_project(start)?;
    let root = discovery.layout.root;
    let cfg = ProjectConfig::load(&root)?;
    let graph = parse_project_lockfile(&root)?;
    let plan = plan_execution_with_config(&graph, Some(&root), Some(&cfg.execution));
    let mut packages = Vec::new();
    for entry in &plan.entries {
        if let Some(dir) = crate::exec_discover::resolve_package_dir_for_discovery(
            &root,
            &entry.package_key,
            entry.name.as_deref(),
        ) {
            if let Ok(d) = crate::exec_discover::discover_package_dir(&dir) {
                packages.push(d);
            }
        }
    }
    Ok((packages, cfg, plan))
}

/// Filter plan entries that are runnable under current policy.
pub fn runnable_entries<'a>(plan: &'a ExecPlan, cfg: &ExecutionConfig) -> Vec<&'a ExecPlanEntry> {
    plan.entries
        .iter()
        .filter(|e| {
            e.would_execute
                && e.name.as_ref().is_some_and(|n| cfg.package_allowed(n))
                && !e.allowed_outputs.is_empty()
        })
        .collect()
}

/// Host + Node ABI identity for execution cache keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecIdentity {
    /// npm-style OS.
    pub os: String,
    /// npm-style CPU.
    pub cpu: String,
    /// Node module ABI.
    pub node_abi: String,
    /// Node version string.
    pub node_version: String,
    /// Execution profile (`offline`).
    pub profile: String,
    /// Allow-scripts digest.
    pub scripts_digest: String,
    /// Declared outputs digest for the package.
    pub outputs_digest: String,
    /// Input package tree digest (pre-exec).
    pub input_digest: String,
}

impl ExecIdentity {
    /// Hex cache key.
    pub fn cache_key(&self) -> String {
        let mut h = Sha256::new();
        h.update(b"weave-exec-v1\0");
        for part in [
            &self.os,
            &self.cpu,
            &self.node_abi,
            &self.node_version,
            &self.profile,
            &self.scripts_digest,
            &self.outputs_digest,
            &self.input_digest,
        ] {
            h.update(part.as_bytes());
            h.update(b"\0");
        }
        hex(&h.finalize())
    }
}

/// Capture Node ABI/version via `node -p`.
pub fn probe_node_identity() -> weave_core::Result<(String, String)> {
    let out = Command::new("node")
        .args([
            "-p",
            "JSON.stringify({abi:process.versions.modules,ver:process.versions.node})",
        ])
        .output()
        .map_err(|source| Error::Io {
            path: PathBuf::from("node"),
            source,
        })?;
    if !out.status.success() {
        return Err(Error::InvalidState {
            path: PathBuf::from("node"),
            reason: format!(
                "failed to probe Node ABI: {}",
                String::from_utf8_lossy(&out.stderr)
            ),
        });
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|err| Error::InvalidState {
            path: PathBuf::from("node"),
            reason: format!("invalid node probe JSON: {err}"),
        })?;
    let abi = v
        .get("abi")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_owned();
    let ver = v
        .get("ver")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_owned();
    if abi.is_empty() {
        return Err(Error::InvalidState {
            path: PathBuf::from("node"),
            reason: "empty Node ABI".into(),
        });
    }
    Ok((abi, ver))
}

/// Digest a directory's regular files (sorted paths) for input identity.
pub fn digest_tree(root: &Path) -> weave_core::Result<String> {
    digest_tree_excluding(root, &[])
}

/// Digest a package tree while ignoring declared output paths.
///
/// Excluding sealed outputs keeps the cache key stable after applying CAS
/// artifacts onto a candidate package directory.
pub fn digest_tree_excluding(root: &Path, exclude: &[String]) -> weave_core::Result<String> {
    let exclude: BTreeSet<String> = exclude.iter().map(|s| s.replace('\\', "/")).collect();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    collect_files(root, root, &exclude, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut h = Sha256::new();
    for (path, bytes) in files {
        h.update(path.as_bytes());
        h.update(b"\0");
        h.update(&bytes);
        h.update(b"\0");
    }
    Ok(hex(&h.finalize()))
}

fn collect_files(
    root: &Path,
    current: &Path,
    exclude: &BTreeSet<String>,
    out: &mut Vec<(String, Vec<u8>)>,
) -> weave_core::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(current)
        .map_err(|source| Error::Io {
            path: current.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| Error::Io {
            path: current.to_path_buf(),
            source,
        })?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        if name == ".weave-seal-stage" || name == ".weave-apply-stage" {
            continue;
        }
        let meta = fs::symlink_metadata(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            collect_files(root, &path, exclude, out)?;
            continue;
        }
        if meta.is_file() {
            let rel = path
                .strip_prefix(root)
                .map_err(|_| Error::MaterializationFailed {
                    path: path.clone(),
                    reason: "path escapes root while digesting".into(),
                })?
                .to_string_lossy()
                .replace('\\', "/");
            if exclude.contains(&rel) {
                continue;
            }
            let bytes = fs::read(&path).map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
            out.push((rel, bytes));
        }
    }
    Ok(())
}

/// Validate a declared relative output path (no traversal / absolute).
pub fn validate_declared_output(rel: &str) -> weave_core::Result<PathBuf> {
    let path = Path::new(rel);
    if path.is_absolute() {
        return Err(Error::MaterializationFailed {
            path: path.to_path_buf(),
            reason: "absolute declared output paths are not allowed".into(),
        });
    }
    for comp in path.components() {
        match comp {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::MaterializationFailed {
                    path: path.to_path_buf(),
                    reason: "path traversal rejected in declared output".into(),
                });
            }
        }
    }
    if rel.is_empty() {
        return Err(Error::MaterializationFailed {
            path: PathBuf::from(rel),
            reason: "empty declared output path".into(),
        });
    }
    Ok(PathBuf::from(rel.replace('\\', "/")))
}

/// Seal only the declared output paths from `work` into the content store.
pub fn seal_declared_outputs(
    store: &ContentStore,
    work: &Path,
    declared: &[String],
) -> weave_core::Result<ExecSealReport> {
    if declared.is_empty() {
        return Err(Error::InvalidState {
            path: work.to_path_buf(),
            reason: "no declared_outputs configured for package — refusing to seal".into(),
        });
    }

    let stage = work.join(".weave-seal-stage");
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(|source| Error::Io {
            path: stage.clone(),
            source,
        })?;
    }
    fs::create_dir_all(&stage).map_err(|source| Error::Io {
        path: stage.clone(),
        source,
    })?;

    let mut sealed_paths = Vec::new();
    for rel in declared {
        let safe = validate_declared_output(rel)?;
        let src = work.join(&safe);
        if !src.is_file() {
            return Err(Error::InvalidState {
                path: src,
                reason: format!("declared output missing after execution: {rel}"),
            });
        }
        // Reject if canonical path escapes work (symlink tricks).
        let canon_work = fs::canonicalize(work).map_err(|source| Error::Io {
            path: work.to_path_buf(),
            source,
        })?;
        let canon_src = fs::canonicalize(&src).map_err(|source| Error::Io {
            path: src.clone(),
            source,
        })?;
        if !canon_src.starts_with(&canon_work) {
            return Err(Error::MaterializationFailed {
                path: src,
                reason: "declared output resolves outside work root".into(),
            });
        }
        let dest = stage.join(&safe);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::copy(&src, &dest).map_err(|source| Error::Io {
            path: dest.clone(),
            source,
        })?;
        sealed_paths.push(safe.to_string_lossy().replace('\\', "/"));
    }

    let tgz = pack_directory_as_npm_tarball(&stage).map_err(|err| Error::InvalidState {
        path: stage.clone(),
        reason: format!("CAS sealing failure: {err}"),
    })?;
    let id = hash_bytes(&tgz);
    store
        .put(&tgz, Some(&id))
        .map_err(|err| Error::InvalidState {
            path: stage,
            reason: format!("CAS sealing failure: {err}"),
        })?;

    let _ = fs::remove_dir_all(work.join(".weave-seal-stage"));

    Ok(ExecSealReport {
        output_artifact_id: id,
        sealed_paths,
        cache_key: String::new(), // filled by caller
    })
}

/// Persisted execution cache index entry (under `$WEAVE_HOME/exec/cache-index/`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecCacheRecord {
    /// Package name.
    pub package: String,
    /// Sealed output artifact id.
    pub output_artifact_id: String,
    /// Cache key (must match filename identity).
    pub cache_key: String,
    /// Paths sealed into the artifact.
    pub sealed_paths: Vec<String>,
    /// Node ABI recorded at seal time.
    pub node_abi: String,
    /// Host OS (npm token).
    pub os: String,
    /// Host CPU (npm token).
    pub cpu: String,
    /// Execution profile.
    pub profile: String,
}

/// Report from integrating sealed execution outputs into a candidate tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecIntegrateReport {
    /// Allowlisted packages considered for this candidate.
    pub packages_considered: usize,
    /// Packages satisfied from CAS cache without re-running scripts.
    pub cache_hits: usize,
    /// Packages that ran sandboxed scripts.
    pub executed: usize,
    /// Packages that received sealed outputs onto the candidate.
    pub applied: usize,
}

/// Build the cache identity for a package input tree.
pub fn build_exec_identity(
    cfg: &ExecutionConfig,
    package: &str,
    input_package_dir: &Path,
) -> weave_core::Result<ExecIdentity> {
    let host = HostPlatform::current();
    let (node_abi, node_version) = probe_node_identity()?;
    let declared = cfg.outputs_for(package).to_vec();
    let input_digest = digest_tree_excluding(input_package_dir, &declared)?;
    let scripts_digest = {
        let mut h = Sha256::new();
        let mut scripts = cfg.allow_scripts.clone();
        scripts.sort();
        for s in scripts {
            h.update(s.as_bytes());
            h.update(b",");
        }
        hex(&h.finalize())
    };
    let outputs_digest = {
        let mut h = Sha256::new();
        let mut outs = declared;
        outs.sort();
        for o in outs {
            h.update(o.as_bytes());
            h.update(b",");
        }
        hex(&h.finalize())
    };
    Ok(ExecIdentity {
        os: host.npm_os().to_owned(),
        cpu: host.npm_cpu().to_owned(),
        node_abi,
        node_version,
        profile: cfg.profile.clone(),
        scripts_digest,
        outputs_digest,
        input_digest,
    })
}

/// Look up a cache index record by key (does not verify store contents).
pub fn lookup_exec_cache(cache_key: &str) -> weave_core::Result<Option<ExecCacheRecord>> {
    let path = weave_home_exec_root()?
        .join("cache-index")
        .join(format!("{cache_key}.json"));
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).map_err(|source| Error::Io {
        path: path.clone(),
        source,
    })?;
    let record: ExecCacheRecord =
        serde_json::from_str(&text).map_err(|err| Error::InvalidState {
            path,
            reason: format!("invalid exec cache index: {err}"),
        })?;
    if record.cache_key != cache_key {
        return Err(Error::InvalidState {
            path: PathBuf::from(format!("exec/cache-index/{cache_key}.json")),
            reason: "cache index cache_key mismatch".into(),
        });
    }
    Ok(Some(record))
}

fn write_exec_cache(record: &ExecCacheRecord) -> weave_core::Result<()> {
    persist_exec_cache(record)
}

/// Persist an execution cache index record (used by runs and tests).
pub fn persist_exec_cache(record: &ExecCacheRecord) -> weave_core::Result<()> {
    let index_dir = weave_home_exec_root()?.join("cache-index");
    fs::create_dir_all(&index_dir).map_err(|source| Error::Io {
        path: index_dir.clone(),
        source,
    })?;
    let index_path = index_dir.join(format!("{}.json", record.cache_key));
    let bytes = serde_json::to_vec_pretty(record).map_err(|err| Error::InvalidState {
        path: index_path.clone(),
        reason: format!("serialize exec cache index: {err}"),
    })?;
    fs::write(&index_path, bytes).map_err(|source| Error::Io {
        path: index_path,
        source,
    })?;
    Ok(())
}

/// Verify a cache record matches the current identity and store contents.
pub fn verify_exec_cache_hit(
    store: &ContentStore,
    identity: &ExecIdentity,
    record: &ExecCacheRecord,
    declared: &[String],
) -> weave_core::Result<ArtifactId> {
    let expected_key = identity.cache_key();
    if record.cache_key != expected_key {
        return Err(Error::InvalidState {
            path: PathBuf::from("exec/cache-index"),
            reason: "execution cache identity mismatch (cache_key)".into(),
        });
    }
    if record.os != identity.os
        || record.cpu != identity.cpu
        || record.node_abi != identity.node_abi
        || record.profile != identity.profile
    {
        return Err(Error::InvalidState {
            path: PathBuf::from("exec/cache-index"),
            reason: "execution cache identity mismatch (platform/ABI/profile)".into(),
        });
    }
    let mut declared_sorted: Vec<String> = declared.iter().map(|s| s.replace('\\', "/")).collect();
    declared_sorted.sort();
    let mut sealed = record.sealed_paths.clone();
    sealed.sort();
    if sealed != declared_sorted {
        return Err(Error::InvalidState {
            path: PathBuf::from("exec/cache-index"),
            reason: "execution cache sealed_paths do not match declared_outputs".into(),
        });
    }
    let id = ArtifactId::parse(&record.output_artifact_id)?;
    if !store.contains(&id) {
        return Err(Error::InvalidState {
            path: PathBuf::from("exec/cache-index"),
            reason: format!("sealed output artifact missing from CAS: {id}"),
        });
    }
    Ok(id)
}

/// Apply a CAS-sealed output artifact onto `package_dir`, copying only declared paths.
///
/// Any path present in the sealed tarball that is not declared is rejected — undeclared
/// outputs must never enter the candidate / activated environment.
pub fn apply_sealed_outputs(
    store: &ContentStore,
    output_artifact_id: &ArtifactId,
    package_dir: &Path,
    declared: &[String],
) -> weave_core::Result<Vec<String>> {
    if declared.is_empty() {
        return Err(Error::InvalidState {
            path: package_dir.to_path_buf(),
            reason: "refusing to apply sealed outputs with empty declared_outputs".into(),
        });
    }
    let mut declared_set: BTreeSet<String> = BTreeSet::new();
    for rel in declared {
        let safe = validate_declared_output(rel)?;
        declared_set.insert(safe.to_string_lossy().replace('\\', "/"));
    }

    let bytes = store.get(output_artifact_id)?;
    let stage = package_dir.join(".weave-apply-stage");
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(|source| Error::Io {
            path: stage.clone(),
            source,
        })?;
    }
    fs::create_dir_all(&stage).map_err(|source| Error::Io {
        path: stage.clone(),
        source,
    })?;
    extract_npm_tarball(&bytes, &stage)?;

    let mut sealed_files: Vec<String> = Vec::new();
    collect_rel_files(&stage, &stage, &mut sealed_files)?;
    sealed_files.sort();
    for rel in &sealed_files {
        if !declared_set.contains(rel) {
            let _ = fs::remove_dir_all(&stage);
            return Err(Error::InvalidState {
                path: package_dir.to_path_buf(),
                reason: format!(
                    "undeclared output in sealed artifact rejected: {rel} \
                     (not in declared_outputs)"
                ),
            });
        }
    }
    for rel in &declared_set {
        if !sealed_files.iter().any(|s| s == rel) {
            let _ = fs::remove_dir_all(&stage);
            return Err(Error::InvalidState {
                path: package_dir.to_path_buf(),
                reason: format!("declared output missing from sealed artifact: {rel}"),
            });
        }
    }

    let mut applied = Vec::new();
    for rel in &declared_set {
        let src = stage.join(rel);
        let dest = package_dir.join(rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::copy(&src, &dest).map_err(|source| Error::Io {
            path: dest.clone(),
            source,
        })?;
        applied.push(rel.clone());
    }
    let _ = fs::remove_dir_all(&stage);
    Ok(applied)
}

fn collect_rel_files(root: &Path, current: &Path, out: &mut Vec<String>) -> weave_core::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(current)
        .map_err(|source| Error::Io {
            path: current.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| Error::Io {
            path: current.to_path_buf(),
            source,
        })?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        if meta.is_dir() {
            collect_rel_files(root, &path, out)?;
        } else if meta.is_file() {
            let rel = path
                .strip_prefix(root)
                .map_err(|_| Error::MaterializationFailed {
                    path: path.clone(),
                    reason: "path escapes apply stage".into(),
                })?
                .to_string_lossy()
                .replace('\\', "/");
            out.push(rel);
        }
    }
    Ok(())
}

/// Refuse inputs under the live project `node_modules` tree.
pub fn refuse_live_node_modules(
    project_root: &Path,
    input_package_dir: &Path,
) -> weave_core::Result<()> {
    let live_nm = project_root.join("node_modules");
    let input_canon = fs::canonicalize(input_package_dir).map_err(|source| Error::Io {
        path: input_package_dir.to_path_buf(),
        source,
    })?;
    if live_nm.exists() {
        if let Ok(live_canon) = fs::canonicalize(&live_nm) {
            if input_canon.starts_with(&live_canon) {
                return Err(Error::InvalidState {
                    path: input_package_dir.to_path_buf(),
                    reason: "refusing to execute against the live node_modules tree (ADR-0018)"
                        .into(),
                });
            }
        }
    }
    Ok(())
}

fn package_dir_under_candidate(candidate_root: &Path, package_name: &str) -> PathBuf {
    candidate_root.join("node_modules").join(package_name)
}

fn default_install_script() -> PathBuf {
    PathBuf::from("scripts/install.js")
}

/// Resolve sealed outputs from cache or sandboxed execution, then apply onto `package_dir`.
///
/// `package_dir` must be under the candidate tree (or another isolated input), never live NM.
pub fn ensure_package_outputs_on_candidate(
    project_root: &Path,
    package: &str,
    package_dir: &Path,
    allow_execute: bool,
) -> weave_core::Result<(ExecSealReport, bool)> {
    let cfg = ProjectConfig::load(project_root)?;
    require_execution_enabled(&cfg.execution)?;
    if !cfg.execution.package_allowed(package) {
        return Err(Error::InvalidState {
            path: PathBuf::from(".weave/config.toml"),
            reason: format!("package {package:?} is not in execution.allow_packages"),
        });
    }
    let declared = cfg.execution.outputs_for(package).to_vec();
    if declared.is_empty() {
        return Err(Error::InvalidState {
            path: PathBuf::from(".weave/config.toml"),
            reason: format!("execution.declared_outputs missing for package {package:?}"),
        });
    }
    for rel in &declared {
        validate_declared_output(rel)?;
    }
    refuse_live_node_modules(project_root, package_dir)?;

    let identity = build_exec_identity(&cfg.execution, package, package_dir)?;
    let cache_key = identity.cache_key();
    let store = ContentStore::open(PathBuf::from(&cfg.store_path))?;

    if let Some(record) = lookup_exec_cache(&cache_key)? {
        match verify_exec_cache_hit(&store, &identity, &record, &declared) {
            Ok(artifact_id) => {
                let applied = apply_sealed_outputs(&store, &artifact_id, package_dir, &declared)?;
                return Ok((
                    ExecSealReport {
                        output_artifact_id: artifact_id,
                        sealed_paths: applied,
                        cache_key,
                    },
                    true,
                ));
            }
            Err(_) => {
                // Stale/mismatched index — fall through.
            }
        }
    }

    // Phase 10: allowlisted prebuild fetch before Bubblewrap, when configured.
    if !cfg.execution.prebuild_fetches_for(package).is_empty() {
        if !allow_execute {
            return Err(Error::InvalidState {
                path: package_dir.to_path_buf(),
                reason: format!(
                    "prebuild/execution cache miss for {package:?} and execution was not permitted"
                ),
            });
        }
        match crate::prebuild_fetch::ensure_prebuild_on_candidate(
            project_root,
            package,
            package_dir,
            &crate::prebuild_fetch::UreqPrebuildTransport::new(),
            /*dry_run*/ false,
        ) {
            Ok(Some(report)) => return Ok((report.seal, report.cache_hit)),
            Ok(None) => {
                // No matching fetch under offline / no selection — fall through only if offline
                // and no fetch required; otherwise ensure_prebuild already erred.
            }
            Err(err) => return Err(err),
        }
    }

    if !allow_execute {
        return Err(Error::InvalidState {
            path: package_dir.to_path_buf(),
            reason: format!("execution cache miss for {package:?} and execution was not permitted"),
        });
    }

    require_sandbox()?;
    let report = exec_run_sandboxed_inner(
        &cfg,
        &ExecRunRequest {
            project_root: project_root.to_path_buf(),
            package: package.to_owned(),
            input_package_dir: package_dir.to_path_buf(),
            script_rel: default_install_script(),
        },
        &identity,
        &declared,
        /*skip_cache*/ true,
    )?;
    apply_sealed_outputs(
        &store,
        &report.seal.output_artifact_id,
        package_dir,
        &declared,
    )?;
    Ok((report.seal, false))
}

/// Integrate allowlisted sealed execution outputs into the candidate tree.
///
/// Called only when the caller already enforced `--with-exec`. Requires
/// `execution.enabled = true`. Never touches the live `node_modules`.
pub fn integrate_execution_into_candidate(
    project_root: &Path,
    candidate_root: &Path,
) -> weave_core::Result<ExecIntegrateReport> {
    let cfg = ProjectConfig::load(project_root)?;
    require_execution_enabled(&cfg.execution)?;

    let live_nm = project_root.join("node_modules");
    let cand_canon = fs::canonicalize(candidate_root).map_err(|source| Error::Io {
        path: candidate_root.to_path_buf(),
        source,
    })?;
    if live_nm.exists() {
        if let Ok(live_canon) = fs::canonicalize(&live_nm) {
            if cand_canon == live_canon || cand_canon.starts_with(&live_canon) {
                return Err(Error::InvalidState {
                    path: candidate_root.to_path_buf(),
                    reason: "refusing to integrate execution into the live node_modules tree"
                        .into(),
                });
            }
        }
    }

    let mut report = ExecIntegrateReport::default();
    for package in &cfg.execution.allow_packages {
        let pkg_dir = package_dir_under_candidate(candidate_root, package);
        if !pkg_dir.is_dir() {
            return Err(Error::InvalidState {
                path: pkg_dir,
                reason: format!(
                    "allowlisted package {package:?} missing from candidate; \
                     cannot apply execution outputs"
                ),
            });
        }
        report.packages_considered += 1;
        let (_seal, cache_hit) =
            ensure_package_outputs_on_candidate(project_root, package, &pkg_dir, true)?;
        if cache_hit {
            report.cache_hits += 1;
        } else {
            report.executed += 1;
        }
        report.applied += 1;
    }
    Ok(report)
}

/// Options for a single package execution.
#[derive(Debug, Clone)]
pub struct ExecRunRequest {
    /// Project root (for config + WEAVE_HOME resolution via store_path).
    pub project_root: PathBuf,
    /// Package name (must be allowlisted).
    pub package: String,
    /// Input package directory (copied into ephemeral work; never live node_modules).
    pub input_package_dir: PathBuf,
    /// Script file relative to package root to run (Phase 7: install entry).
    pub script_rel: PathBuf,
}

/// Run one allowlisted package install script under Bubblewrap (offline).
///
/// Cache hits return the sealed artifact without re-running the script.
pub fn exec_run_sandboxed(req: &ExecRunRequest) -> weave_core::Result<ExecRunReport> {
    let cfg = ProjectConfig::load(&req.project_root)?;
    require_execution_enabled(&cfg.execution)?;

    if !cfg.execution.package_allowed(&req.package) {
        return Err(Error::InvalidState {
            path: PathBuf::from(".weave/config.toml"),
            reason: format!(
                "package {:?} is not in execution.allow_packages",
                req.package
            ),
        });
    }
    if !cfg.execution.allow_scripts.iter().any(|s| s == "install") {
        return Err(Error::InvalidState {
            path: PathBuf::from(".weave/config.toml"),
            reason: "execution.allow_scripts must include \"install\" for Phase 7 runs".into(),
        });
    }

    let declared = cfg.execution.outputs_for(&req.package).to_vec();
    if declared.is_empty() {
        return Err(Error::InvalidState {
            path: PathBuf::from(".weave/config.toml"),
            reason: format!(
                "execution.declared_outputs missing for package {:?}",
                req.package
            ),
        });
    }
    for rel in &declared {
        validate_declared_output(rel)?;
    }

    refuse_live_node_modules(&req.project_root, &req.input_package_dir)?;
    let identity = build_exec_identity(&cfg.execution, &req.package, &req.input_package_dir)?;
    let store = ContentStore::open(PathBuf::from(&cfg.store_path))?;
    let cache_key = identity.cache_key();

    if let Some(record) = lookup_exec_cache(&cache_key)? {
        if let Ok(artifact_id) = verify_exec_cache_hit(&store, &identity, &record, &declared) {
            return Ok(ExecRunReport {
                package: req.package.clone(),
                work_root: weave_home_exec_root()?.join("cache-hit"),
                seal: ExecSealReport {
                    output_artifact_id: artifact_id,
                    sealed_paths: record.sealed_paths,
                    cache_key,
                },
                node_abi: identity.node_abi.clone(),
                platform_os: identity.os.clone(),
                platform_cpu: identity.cpu.clone(),
            });
        }
    }

    require_sandbox()?;
    exec_run_sandboxed_inner(&cfg, req, &identity, &declared, /*skip_cache*/ true)
}

fn exec_run_sandboxed_inner(
    cfg: &ProjectConfig,
    req: &ExecRunRequest,
    identity: &ExecIdentity,
    declared: &[String],
    _skip_cache: bool,
) -> weave_core::Result<ExecRunReport> {
    let cache_key = identity.cache_key();
    let node_abi = identity.node_abi.clone();

    let run_id = format!(
        "{}-{}",
        &cache_key[..16.min(cache_key.len())],
        std::process::id()
    );
    let exec_root = weave_home_exec_root()?.join(&run_id);
    if exec_root.exists() {
        fs::remove_dir_all(&exec_root).map_err(|source| Error::Io {
            path: exec_root.clone(),
            source,
        })?;
    }
    let work = exec_root.join("work");
    let home = exec_root.join("home");
    let log_dir = exec_root.join("log");
    fs::create_dir_all(&work).map_err(|source| Error::Io {
        path: work.clone(),
        source,
    })?;
    fs::create_dir_all(&home).map_err(|source| Error::Io {
        path: home.clone(),
        source,
    })?;
    fs::create_dir_all(&log_dir).map_err(|source| Error::Io {
        path: log_dir.clone(),
        source,
    })?;

    copy_dir_recursive(&req.input_package_dir, &work)?;

    let script = work.join(&req.script_rel);
    if !script.is_file() {
        return Err(Error::InvalidState {
            path: script,
            reason: "install script missing in package input".into(),
        });
    }

    let status = run_bwrap_offline(&work, &home, &req.script_rel)?;
    let log_path = log_dir.join("stdout.txt");
    fs::write(&log_path, &status.stdout).map_err(|source| Error::Io {
        path: log_path.clone(),
        source,
    })?;
    fs::write(log_dir.join("stderr.txt"), &status.stderr).map_err(|source| Error::Io {
        path: log_dir.join("stderr.txt"),
        source,
    })?;

    if !status.success {
        return Err(Error::InvalidState {
            path: exec_root,
            reason: format!(
                "sandboxed execution failed (exit {}): {}",
                status.code,
                String::from_utf8_lossy(&status.stderr)
            ),
        });
    }

    let store = ContentStore::open(PathBuf::from(&cfg.store_path))?;
    let mut seal = seal_declared_outputs(&store, &work, declared)?;
    seal.cache_key = cache_key.clone();

    let record = ExecCacheRecord {
        package: req.package.clone(),
        output_artifact_id: seal.output_artifact_id.to_string(),
        cache_key: cache_key.clone(),
        sealed_paths: seal.sealed_paths.clone(),
        node_abi: node_abi.clone(),
        os: identity.os.clone(),
        cpu: identity.cpu.clone(),
        profile: identity.profile.clone(),
    };
    let _ = write_exec_cache(&record);

    Ok(ExecRunReport {
        package: req.package.clone(),
        work_root: exec_root,
        seal,
        node_abi,
        platform_os: identity.os.clone(),
        platform_cpu: identity.cpu.clone(),
    })
}

struct BwrapStatus {
    success: bool,
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_bwrap_offline(
    work: &Path,
    home: &Path,
    script_rel: &Path,
) -> weave_core::Result<BwrapStatus> {
    let node = which_node()?;
    let mut cmd = Command::new(bwrap_bin());
    cmd.arg("--die-with-parent");
    cmd.arg("--unshare-net");
    cmd.arg("--unshare-pid");
    cmd.arg("--proc").arg("/proc");
    cmd.arg("--dev").arg("/dev");
    cmd.arg("--tmpfs").arg("/tmp");
    // Read-only host paths commonly needed for Node.
    for p in ["/usr", "/bin", "/lib", "/lib64", "/etc/alternatives"] {
        if Path::new(p).exists() {
            cmd.arg("--ro-bind").arg(p).arg(p);
        }
    }
    // Node may live under /usr/local or nvm — bind its parent read-only.
    if let Some(parent) = node.parent() {
        if parent.starts_with("/home")
            || parent.starts_with("/opt")
            || parent.starts_with("/usr/local")
        {
            cmd.arg("--ro-bind").arg(parent).arg(parent);
        }
    }
    cmd.arg("--bind").arg(work).arg("/work");
    cmd.arg("--bind").arg(home).arg("/home/weave");
    cmd.arg("--chdir").arg("/work");
    cmd.arg("--setenv").arg("HOME").arg("/home/weave");
    cmd.arg("--setenv").arg("TMPDIR").arg("/tmp");
    cmd.arg("--setenv")
        .arg("PATH")
        .arg("/usr/bin:/bin:/usr/local/bin");
    cmd.arg("--").arg(&node).arg(script_rel);

    let output = cmd.output().map_err(|source| Error::Io {
        path: bwrap_bin(),
        source,
    })?;
    Ok(BwrapStatus {
        success: output.status.success(),
        code: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

fn which_node() -> weave_core::Result<PathBuf> {
    if let Ok(p) = std::env::var("WEAVE_NODE_PATH") {
        return Ok(PathBuf::from(p));
    }
    let out = Command::new("sh")
        .args(["-c", "command -v node"])
        .output()
        .map_err(|source| Error::Io {
            path: PathBuf::from("node"),
            source,
        })?;
    if !out.status.success() {
        return Err(Error::InvalidState {
            path: PathBuf::from("node"),
            reason: "node binary not found on PATH".into(),
        });
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    Ok(PathBuf::from(path))
}

fn weave_home_exec_root() -> weave_core::Result<PathBuf> {
    let home = weave_store::default_weave_home()?;
    Ok(home.join("exec"))
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> weave_core::Result<()> {
    fs::create_dir_all(dst).map_err(|source| Error::Io {
        path: dst.to_path_buf(),
        source,
    })?;
    for entry in fs::read_dir(src).map_err(|source| Error::Io {
        path: src.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| Error::Io {
            path: src.to_path_buf(),
            source,
        })?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let meta = fs::symlink_metadata(&from).map_err(|source| Error::Io {
            path: from.clone(),
            source,
        })?;
        if meta.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if meta.is_file() {
            fs::copy(&from, &to).map_err(|source| Error::Io {
                path: to.clone(),
                source,
            })?;
        }
    }
    Ok(())
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
    use crate::test_util::lock_weave_home;
    use std::collections::BTreeMap;
    use std::io::Write;

    fn write_exec_gen_pkg(dir: &Path) {
        fs::create_dir_all(dir.join("scripts")).unwrap();
        fs::write(
            dir.join("package.json"),
            r#"{"name":"exec-gen","version":"1.0.0","scripts":{"install":"node scripts/install.js"}}"#,
        )
        .unwrap();
        fs::write(
            dir.join("scripts/install.js"),
            br#"const fs = require("fs");
fs.mkdirSync("generated", { recursive: true });
fs.writeFileSync("generated/hello.txt", "weave-exec-ok\n");
"#,
        )
        .unwrap();
    }

    fn write_project(root: &Path, enabled: bool, outputs: &[&str]) {
        fs::create_dir_all(root.join(".weave")).unwrap();
        let mut declared = BTreeMap::new();
        declared.insert(
            "exec-gen".into(),
            outputs.iter().map(|s| (*s).to_owned()).collect(),
        );
        let cfg = ProjectConfig {
            version: 1,
            store_path: root.join("store").display().to_string(),
            materialization_version: "4".into(),
            execution: ExecutionConfig {
                enabled,
                profile: "offline".into(),
                allow_packages: vec!["exec-gen".into()],
                allow_scripts: vec!["install".into()],
                declared_outputs: declared,
                allow_weak_sandbox: false,
                prebuild: Default::default(),
            },
        };
        fs::create_dir_all(root.join("store")).unwrap();
        let mut f = fs::File::create(root.join(".weave/config.toml")).unwrap();
        f.write_all(toml::to_string_pretty(&cfg).unwrap().as_bytes())
            .unwrap();
    }

    #[test]
    fn disabled_execution_is_rejected() {
        let _g = lock_weave_home();
        let tmp = tempfile::tempdir().unwrap();
        write_project(tmp.path(), false, &["generated/hello.txt"]);
        let cfg = ProjectConfig::load(tmp.path()).unwrap();
        let err = require_execution_enabled(&cfg.execution).unwrap_err();
        assert!(err.to_string().contains("disabled"));
    }

    #[test]
    fn env_var_cannot_enable_execution() {
        let _g = lock_weave_home();
        std::env::set_var("WEAVE_EXEC", "1");
        let cfg = ExecutionConfig {
            enabled: false,
            ..ExecutionConfig::default()
        };
        assert!(require_execution_enabled(&cfg).is_err());
        std::env::remove_var("WEAVE_EXEC");
    }

    #[test]
    fn sandbox_unavailable_fails_closed() {
        let _g = lock_weave_home();
        let prev = std::env::var_os("WEAVE_BWRAP_PATH");
        std::env::set_var("WEAVE_BWRAP_PATH", "/nonexistent/bwrap-binary");
        let err = require_sandbox().unwrap_err();
        assert!(err.to_string().contains("sandbox unavailable"));
        match prev {
            Some(v) => std::env::set_var("WEAVE_BWRAP_PATH", v),
            None => std::env::remove_var("WEAVE_BWRAP_PATH"),
        }
    }

    #[test]
    fn missing_declared_output_is_cas_seal_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("work");
        fs::create_dir_all(&work).unwrap();
        let store = ContentStore::open(tmp.path().join("store")).unwrap();
        let err =
            seal_declared_outputs(&store, &work, &["generated/hello.txt".into()]).unwrap_err();
        assert!(
            err.to_string().contains("missing") || err.to_string().contains("declared"),
            "{err}"
        );
    }

    #[test]
    fn undeclared_output_path_rejected_for_seal() {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("work");
        fs::create_dir_all(&work).unwrap();
        fs::write(work.join("secret.txt"), b"x").unwrap();
        let store = ContentStore::open(tmp.path().join("store")).unwrap();
        // Attempting to seal a path not listed should use only declared list —
        // empty declared fails; sealing undeclared via API requires listing it.
        // Security: validate_declared_output rejects traversal; seal refuses empty.
        assert!(seal_declared_outputs(&store, &work, &[]).is_err());
        assert!(validate_declared_output("../etc/passwd").is_err());
        assert!(validate_declared_output("/abs").is_err());
    }

    #[test]
    fn path_traversal_declared_output_rejected() {
        assert!(validate_declared_output("foo/../../x").is_err());
        assert!(validate_declared_output("..").is_err());
    }

    #[test]
    fn cache_identity_changes_with_abi_and_outputs() {
        let a = ExecIdentity {
            os: "linux".into(),
            cpu: "x64".into(),
            node_abi: "137".into(),
            node_version: "24.0.0".into(),
            profile: "offline".into(),
            scripts_digest: "s1".into(),
            outputs_digest: "o1".into(),
            input_digest: "i1".into(),
        };
        let mut b = a.clone();
        b.node_abi = "108".into();
        assert_ne!(a.cache_key(), b.cache_key());
        let mut c = a.clone();
        c.outputs_digest = "o2".into();
        assert_ne!(a.cache_key(), c.cache_key());
    }

    #[test]
    fn failed_execution_does_not_seal() {
        let _g = lock_weave_home();
        if !bwrap_available() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
        write_project(tmp.path(), true, &["generated/hello.txt"]);
        let pkg = tmp.path().join("pkg");
        write_exec_gen_pkg(&pkg);
        fs::write(
            pkg.join("scripts/install.js"),
            b"console.error('boom'); process.exit(1);\n",
        )
        .unwrap();
        let err = exec_run_sandboxed(&ExecRunRequest {
            project_root: tmp.path().to_path_buf(),
            package: "exec-gen".into(),
            input_package_dir: pkg,
            script_rel: PathBuf::from("scripts/install.js"),
        })
        .unwrap_err();
        assert!(err.to_string().contains("failed"));
        std::env::remove_var("WEAVE_HOME");
    }

    #[test]
    fn digest_excluding_declared_is_stable_after_apply() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg = tmp.path().join("pkg");
        fs::create_dir_all(pkg.join("scripts")).unwrap();
        fs::write(pkg.join("package.json"), b"{}").unwrap();
        fs::write(pkg.join("scripts/install.js"), b"x").unwrap();
        let before = digest_tree_excluding(&pkg, &["generated/hello.txt".into()]).unwrap();
        fs::create_dir_all(pkg.join("generated")).unwrap();
        fs::write(pkg.join("generated/hello.txt"), b"out\n").unwrap();
        let after = digest_tree_excluding(&pkg, &["generated/hello.txt".into()]).unwrap();
        assert_eq!(before, after);
        assert_ne!(digest_tree(&pkg).unwrap(), before);
    }

    #[test]
    fn apply_rejects_undeclared_paths_in_seal() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::open(tmp.path().join("store")).unwrap();
        let stage = tmp.path().join("stage");
        fs::create_dir_all(stage.join("generated")).unwrap();
        fs::write(stage.join("generated/hello.txt"), b"ok\n").unwrap();
        fs::write(stage.join("secret.txt"), b"nope\n").unwrap();
        let tgz = pack_directory_as_npm_tarball(&stage).unwrap();
        let id = hash_bytes(&tgz);
        store.put(&tgz, Some(&id)).unwrap();

        let pkg = tmp.path().join("pkg");
        fs::create_dir_all(&pkg).unwrap();
        let err =
            apply_sealed_outputs(&store, &id, &pkg, &["generated/hello.txt".into()]).unwrap_err();
        assert!(err.to_string().contains("undeclared"), "{err}");
        assert!(!pkg.join("secret.txt").exists());
        assert!(!pkg.join("generated/hello.txt").exists());
    }

    #[test]
    fn apply_copies_only_declared_outputs() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::open(tmp.path().join("store")).unwrap();
        let stage = tmp.path().join("stage");
        fs::create_dir_all(stage.join("generated")).unwrap();
        fs::write(stage.join("generated/hello.txt"), b"weave-exec-ok\n").unwrap();
        let tgz = pack_directory_as_npm_tarball(&stage).unwrap();
        let id = hash_bytes(&tgz);
        store.put(&tgz, Some(&id)).unwrap();

        let pkg = tmp.path().join("pkg");
        fs::create_dir_all(&pkg).unwrap();
        let applied =
            apply_sealed_outputs(&store, &id, &pkg, &["generated/hello.txt".into()]).unwrap();
        assert_eq!(applied, vec!["generated/hello.txt".to_string()]);
        assert_eq!(
            fs::read_to_string(pkg.join("generated/hello.txt")).unwrap(),
            "weave-exec-ok\n"
        );
    }

    #[test]
    fn platform_abi_mismatch_fails_verify() {
        let identity = ExecIdentity {
            os: "linux".into(),
            cpu: "x64".into(),
            node_abi: "137".into(),
            node_version: "24.0.0".into(),
            profile: "offline".into(),
            scripts_digest: "s1".into(),
            outputs_digest: "o1".into(),
            input_digest: "i1".into(),
        };
        let record = ExecCacheRecord {
            package: "exec-gen".into(),
            output_artifact_id: "a".repeat(64),
            cache_key: identity.cache_key(),
            sealed_paths: vec!["generated/hello.txt".into()],
            node_abi: "108".into(), // mismatch
            os: identity.os.clone(),
            cpu: identity.cpu.clone(),
            profile: identity.profile.clone(),
        };
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::open(tmp.path().join("store")).unwrap();
        let err =
            verify_exec_cache_hit(&store, &identity, &record, &["generated/hello.txt".into()])
                .unwrap_err();
        assert!(err.to_string().contains("identity mismatch"), "{err}");
    }

    #[test]
    fn refuses_live_node_modules_input() {
        let _g = lock_weave_home();
        if !bwrap_available() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
        write_project(tmp.path(), true, &["generated/hello.txt"]);
        let nm = tmp.path().join("node_modules/exec-gen");
        write_exec_gen_pkg(&nm);
        let err = exec_run_sandboxed(&ExecRunRequest {
            project_root: tmp.path().to_path_buf(),
            package: "exec-gen".into(),
            input_package_dir: nm,
            script_rel: PathBuf::from("scripts/install.js"),
        })
        .unwrap_err();
        assert!(err.to_string().contains("live node_modules"));
        std::env::remove_var("WEAVE_HOME");
    }
}
