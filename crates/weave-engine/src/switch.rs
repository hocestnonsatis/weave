//! Prepare and activate dependency environments (Milestones 5–7, Phase 8).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use weave_core::{
    Error, HostPlatform, PackageKey, PeerAuditStatus, WEAVE_CANDIDATE_DIR, WEAVE_DIR,
};
use weave_fs::{
    activate_candidate, materialize_plan, validate_candidate, ActivationReport,
    MaterializationPlan, MaterializeReport,
};
use weave_lockfile::parse_lockfile;
use weave_store::{ArtifactId, ContentStore};

use crate::acquire::{prepare_artifacts_for_platform, ArtifactSource, DefaultArtifactSource};
use crate::config::ProjectConfig;
use crate::environment::{
    create_environment_with_opts, CreateEnvironmentOpts, EnvironmentId, EnvironmentRecord,
    EnvironmentStore, PlatformIdentity,
};
use crate::exec::{integrate_execution_into_candidate, ExecIntegrateReport};
use crate::project::discover_project;

/// Options controlling prepare / switch behavior.
#[derive(Debug, Clone, Default)]
pub struct SwitchOptions {
    /// When true, run opt-in sandboxed execution against the candidate and apply
    /// CAS-sealed declared outputs before activation.
    ///
    /// Requires `[execution] enabled = true` in `.weave/config.toml`. Plain
    /// `weave switch` keeps this false — execution stays off.
    pub with_exec: bool,
    /// Optional explicit owner/session stamped onto the environment record.
    /// Never inferred — callers (agents) must pass it deliberately.
    pub owner: Option<String>,
}

/// Outcome of preparing (acquiring + materializing) an environment candidate.
#[derive(Debug, Clone)]
pub struct PrepareOutcome {
    /// Environment metadata (saved).
    pub environment: EnvironmentRecord,
    /// Materialization report for the candidate tree.
    pub materialize: MaterializeReport,
    /// Candidate root (`.weave/candidate`).
    pub candidate_root: PathBuf,
    /// Artifacts reused from the content store.
    pub reused_artifacts: usize,
    /// Artifacts newly fetched into the store.
    pub fetched_artifacts: usize,
    /// Execution integration report (empty when `--with-exec` was not used).
    pub execution: ExecIntegrateReport,
}

/// Outcome of a full switch (prepare + activate).
#[derive(Debug, Clone)]
pub struct SwitchOutcome {
    /// Prepare stage result.
    pub prepare: PrepareOutcome,
    /// Activation result.
    pub activation: ActivationReport,
}

/// Materialize the current lockfile into `.weave/candidate` without activating.
pub fn materialize_project(start: &Path) -> weave_core::Result<PrepareOutcome> {
    materialize_project_with_options(start, &SwitchOptions::default())
}

/// Materialize with explicit options (e.g. `--with-exec`).
pub fn materialize_project_with_options(
    start: &Path,
    options: &SwitchOptions,
) -> weave_core::Result<PrepareOutcome> {
    let discovery = discover_project(start)?;
    let source = DefaultArtifactSource::for_project(&discovery.layout.root);
    materialize_project_with_source_options(start, &source, options)
}

/// Like [`materialize_project`] but with a custom artifact source (tests/offline).
pub fn materialize_project_with_source<S: ArtifactSource>(
    start: &Path,
    source: &S,
) -> weave_core::Result<PrepareOutcome> {
    materialize_project_with_source_options(start, source, &SwitchOptions::default())
}

/// Materialize with a custom source and switch options.
pub fn materialize_project_with_source_options<S: ArtifactSource>(
    start: &Path,
    source: &S,
    options: &SwitchOptions,
) -> weave_core::Result<PrepareOutcome> {
    let ctx = SwitchContext::load(start)?;
    ctx.prepare(source, options)
}

/// Prepare and atomically activate the environment for the current lockfile.
///
/// If `target` is `Some`, an existing environment matching that label or id
/// prefix must already match the current lockfile graph. Weave does not run
/// `git switch` automatically.
///
/// Plain switch never executes lifecycle scripts. Pass
/// [`SwitchOptions::with_exec`] only together with config `execution.enabled`.
pub fn switch_project(start: &Path, target: Option<&str>) -> weave_core::Result<SwitchOutcome> {
    switch_project_with_options(start, target, &SwitchOptions::default())
}

/// Switch with explicit options (`--with-exec`).
pub fn switch_project_with_options(
    start: &Path,
    target: Option<&str>,
    options: &SwitchOptions,
) -> weave_core::Result<SwitchOutcome> {
    let discovery = discover_project(start)?;
    let source = DefaultArtifactSource::for_project(&discovery.layout.root);
    switch_project_with_source_options(start, target, &source, options)
}

/// Like [`switch_project`] with a custom artifact source.
pub fn switch_project_with_source<S: ArtifactSource>(
    start: &Path,
    target: Option<&str>,
    source: &S,
) -> weave_core::Result<SwitchOutcome> {
    switch_project_with_source_options(start, target, source, &SwitchOptions::default())
}

/// Switch with a custom source and options.
pub fn switch_project_with_source_options<S: ArtifactSource>(
    start: &Path,
    target: Option<&str>,
    source: &S,
    options: &SwitchOptions,
) -> weave_core::Result<SwitchOutcome> {
    let ctx = SwitchContext::load(start)?;
    if let Some(target) = target {
        ctx.require_target_compatible(target)?;
    }
    let prepare = ctx.prepare(source, options)?;
    let artifacts = artifact_map_from_record(&prepare.environment);
    let host = HostPlatform::current();
    let plan = MaterializationPlan::from_graph_for_platform(&ctx.graph, &artifacts, &host);
    validate_candidate(&plan, &prepare.candidate_root)?;
    let activation = activate_candidate(&ctx.root)?;
    let store = EnvironmentStore::open(&ctx.root);
    store.set_active(&prepare.environment.id)?;
    let environment = store.mark_activated(&prepare.environment.id)?;
    let mut prepare = prepare;
    prepare.environment = environment;
    Ok(SwitchOutcome {
        prepare,
        activation,
    })
}

struct SwitchContext {
    root: PathBuf,
    graph: weave_core::DependencyGraph,
}

impl SwitchContext {
    fn load(start: &Path) -> weave_core::Result<Self> {
        let discovery = discover_project(start)?;
        if !discovery.layout.weave_initialized {
            return Err(Error::NotInitialized {
                root: discovery.layout.root,
            });
        }
        let lockfile =
            discovery
                .layout
                .lockfile
                .as_ref()
                .ok_or_else(|| Error::MissingLockfile {
                    root: discovery.layout.root.clone(),
                })?;
        let graph = parse_lockfile(lockfile)?;
        Ok(Self {
            root: discovery.layout.root,
            graph,
        })
    }

    fn require_target_compatible(&self, target: &str) -> weave_core::Result<()> {
        let store = EnvironmentStore::open(&self.root);
        let envs = store.list()?;
        let found = envs.iter().find(|e| {
            e.label.as_deref() == Some(target)
                || e.id.as_str() == target
                || e.id.as_str().starts_with(target)
        });
        match found {
            Some(env) => {
                let current_id = EnvironmentId::derive(&self.graph, &PlatformIdentity::host());
                if env.id != current_id {
                    return Err(Error::InvalidState {
                        path: self.root.join(WEAVE_DIR),
                        reason: format!(
                            "target '{target}' refers to environment {}, but the current \
                             lockfile produces {}. Checkout the matching branch/lockfile \
                             (Weave does not run git switch automatically).",
                            env.id, current_id
                        ),
                    });
                }
                Ok(())
            }
            None => Err(Error::InvalidState {
                path: self.root.join(WEAVE_DIR),
                reason: format!(
                    "no environment matches target '{target}'. Run `weave env create` \
                     after checking out that branch, or omit the target to use the \
                     current lockfile."
                ),
            }),
        }
    }

    fn prepare<S: ArtifactSource>(
        &self,
        source: &S,
        options: &SwitchOptions,
    ) -> weave_core::Result<PrepareOutcome> {
        if options.with_exec {
            let cfg = ProjectConfig::load(&self.root)?;
            if !cfg.execution.is_enabled() {
                return Err(Error::InvalidState {
                    path: self.root.join(WEAVE_DIR).join("config.toml"),
                    reason: "--with-exec requires [execution] enabled = true in \
                             .weave/config.toml (env vars cannot enable execution)"
                        .into(),
                });
            }
        }

        let config = ProjectConfig::load(&self.root)?;
        let store_root = PathBuf::from(&config.store_path);
        let content = ContentStore::open(&store_root)?;
        let host = HostPlatform::current();

        validate_peer_semantics(&self.graph)?;

        let (stored, _filter) =
            prepare_artifacts_for_platform(&self.graph, source, &content, &host)?;
        let mut reused = 0usize;
        let mut fetched = 0usize;
        let mut artifacts = BTreeMap::new();
        for item in &stored {
            if item.newly_stored {
                fetched += 1;
            } else {
                reused += 1;
            }
            artifacts.insert(item.request.key.clone(), item.artifact_id.clone());
        }

        let label = weave_git::GitRepository::inspect(&self.root)
            .ok()
            .and_then(|r| r.branch);
        let mut record = create_environment_with_opts(
            &self.root,
            &self.graph,
            &artifacts,
            CreateEnvironmentOpts {
                label,
                owner: options.owner.clone(),
            },
        )?;
        record.artifacts = artifacts
            .iter()
            .map(|(k, v)| (k.as_str().to_owned(), v.to_string()))
            .collect();
        EnvironmentStore::open(&self.root).save(&record)?;

        let candidate_root = self.root.join(WEAVE_DIR).join(WEAVE_CANDIDATE_DIR);
        if candidate_root.exists() {
            fs::remove_dir_all(&candidate_root).map_err(|source| Error::Io {
                path: candidate_root.clone(),
                source,
            })?;
        }
        fs::create_dir_all(&candidate_root).map_err(|source| Error::Io {
            path: candidate_root.clone(),
            source,
        })?;

        let plan = MaterializationPlan::from_graph_for_platform(&self.graph, &artifacts, &host);
        let materialize = materialize_plan(&plan, &content, &candidate_root, &self.root)?;

        // Execution only against the isolated candidate, never live node_modules.
        // Plain switch (with_exec=false) skips this entirely — even if config enabled.
        let execution = if options.with_exec {
            integrate_execution_into_candidate(&self.root, &candidate_root)?
        } else {
            ExecIntegrateReport::default()
        };

        validate_candidate(&plan, &candidate_root)?;

        Ok(PrepareOutcome {
            environment: record,
            materialize,
            candidate_root,
            reused_artifacts: reused,
            fetched_artifacts: fetched,
            execution,
        })
    }
}

/// Fail when required peerDependencies are missing from the lockfile graph.
fn validate_peer_semantics(graph: &weave_core::DependencyGraph) -> weave_core::Result<()> {
    let missing: Vec<_> = graph
        .audit_peers()
        .into_iter()
        .filter(|f| matches!(f.status, PeerAuditStatus::MissingRequired))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let detail = missing
        .iter()
        .map(|f| format!("{} requires peer {} ({})", f.package, f.peer, f.requested))
        .collect::<Vec<_>>()
        .join("; ");
    Err(Error::InvalidState {
        path: PathBuf::from("package-lock.json"),
        reason: format!(
            "unsatisfied required peerDependencies: {detail}. \
             Weave does not auto-install peers; ensure the lockfile includes them \
             (npm install) or mark them optional via peerDependenciesMeta."
        ),
    })
}

fn artifact_map_from_record(record: &EnvironmentRecord) -> BTreeMap<PackageKey, ArtifactId> {
    let mut out = BTreeMap::new();
    for (key, id) in &record.artifacts {
        if let Ok(artifact) = ArtifactId::parse(id) {
            out.insert(PackageKey::new(key.clone()), artifact);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    use sha2::{Digest, Sha256};
    use weave_core::{WEAVE_CONFIG, WEAVE_METADATA_DIR};

    use crate::acquire::FileArtifactSource;
    use crate::init::init_project;
    use crate::test_util::lock_weave_home;

    fn setup_project(dir: &Path) {
        let status = Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success());
        let _ = Command::new("git")
            .args(["config", "user.email", "weave@example.com"])
            .current_dir(dir)
            .status();
        let _ = Command::new("git")
            .args(["config", "user.name", "Weave Test"])
            .current_dir(dir)
            .status();

        fs::write(
            dir.join("package.json"),
            r#"{"name":"demo","version":"1.0.0","dependencies":{"demo-pkg":"1.0.0"}}"#,
        )
        .unwrap();
        fs::write(
            dir.join("package-lock.json"),
            r#"{
  "name": "demo",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "packages": {
    "": {
      "name": "demo",
      "version": "1.0.0",
      "dependencies": { "demo-pkg": "1.0.0" }
    },
    "node_modules/demo-pkg": {
      "version": "1.0.0",
      "resolved": "https://example.invalid/demo-pkg/-/demo-pkg-1.0.0.tgz",
      "integrity": "sha256-PLACEHOLDER"
    }
  }
}"#,
        )
        .unwrap();
        fs::write(dir.join("README"), "x").unwrap();
        let status = Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success());
        let status = Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn switch_materializes_and_activates_offline() {
        let _guard = lock_weave_home();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
        let project = tmp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        setup_project(&project);

        let tgz = weave_fs::pack_npm_tarball(&[
            ("package.json", br#"{"name":"demo-pkg","version":"1.0.0"}"#),
            ("index.js", b"module.exports = 42;\n"),
        ]);
        let digest = Sha256::digest(&tgz);
        let integrity = format!("sha256-{}", b64(&digest));
        let lock = fs::read_to_string(project.join("package-lock.json")).unwrap();
        fs::write(
            project.join("package-lock.json"),
            lock.replace("sha256-PLACEHOLDER", &integrity),
        )
        .unwrap();

        init_project(&project).unwrap();
        assert!(project.join(WEAVE_DIR).join(WEAVE_CONFIG).is_file());

        let tarball_path = tmp.path().join("demo-pkg.tgz");
        fs::write(&tarball_path, &tgz).unwrap();
        let source = FileArtifactSource::new(tmp.path()).with_override("demo-pkg", tarball_path);

        let outcome = switch_project_with_source(&project, None, &source).unwrap();
        assert_eq!(outcome.prepare.fetched_artifacts, 1);
        assert!(project.join("node_modules/demo-pkg/index.js").is_file());
        assert_eq!(
            fs::read_to_string(project.join("node_modules/demo-pkg/index.js")).unwrap(),
            "module.exports = 42;\n"
        );
        assert_eq!(outcome.prepare.execution.packages_considered, 0);

        let env_store = EnvironmentStore::open(&project);
        assert!(env_store.active_id().unwrap().is_some());
        assert!(project
            .join(WEAVE_DIR)
            .join(WEAVE_METADATA_DIR)
            .join("active")
            .is_file());

        let again = switch_project_with_source(&project, None, &source).unwrap();
        assert!(again.activation.replaced_existing);
        assert_eq!(again.prepare.reused_artifacts, 1);
        assert_eq!(again.prepare.fetched_artifacts, 0);

        std::env::remove_var("WEAVE_HOME");
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
}
