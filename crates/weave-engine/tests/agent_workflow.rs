//! Phase 18: agent-native workflow integration (offline).
//!
//! Simulates 4–8 concurrent agent project roots sharing one WEAVE_HOME / CAS
//! on the same lockfile graph. Weave never auto-detects agents — owners are
//! explicit. No exec, no network.

use std::fs;
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use sha2::{Digest, Sha256};
use weave_engine::{
    env_create_with_opts, env_list_entries, env_prune, env_remove, init_project, project_status,
    switch_project_with_source, switch_project_with_source_options, EnvCreateOpts, EnvPruneOpts,
    EnvironmentRecord, EnvironmentStore, FileArtifactSource, PlatformIdentity, SwitchOptions,
};
use weave_fs::pack_npm_tarball;
use weave_lockfile::parse_lockfile;

static WEAVE_HOME_LOCK: Mutex<()> = Mutex::new(());

fn lock_home() -> MutexGuard<'static, ()> {
    WEAVE_HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner())
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

fn sri(bytes: &[u8]) -> String {
    format!("sha256-{}", b64(&Sha256::digest(bytes)))
}

fn git_init(dir: &std::path::Path) {
    assert!(Command::new("git")
        .args(["init"])
        .current_dir(dir)
        .status()
        .unwrap()
        .success());
    let _ = Command::new("git")
        .args(["config", "user.email", "weave@test"])
        .current_dir(dir)
        .status();
    let _ = Command::new("git")
        .args(["config", "user.name", "weave"])
        .current_dir(dir)
        .status();
    assert!(Command::new("git")
        .args(["add", "."])
        .current_dir(dir)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["commit", "-m", "init"])
        .current_dir(dir)
        .status()
        .unwrap()
        .success());
}

fn write_tiny_project(project: &std::path::Path, integrity: &str) {
    fs::create_dir_all(project).unwrap();
    fs::write(
        project.join("package.json"),
        r#"{"name":"agent-app","version":"1.0.0","dependencies":{"demo-lib":"1.0.0"}}"#,
    )
    .unwrap();
    let lock = format!(
        r#"{{
  "name": "agent-app",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "requires": true,
  "packages": {{
    "": {{
      "name": "agent-app",
      "version": "1.0.0",
      "dependencies": {{ "demo-lib": "1.0.0" }}
    }},
    "node_modules/demo-lib": {{
      "version": "1.0.0",
      "resolved": "https://example.invalid/demo-lib/-/demo-lib-1.0.0.tgz",
      "integrity": "{integrity}"
    }}
  }}
}}"#
    );
    fs::write(project.join("package-lock.json"), lock).unwrap();
    git_init(project);
}

fn file_source(tarball_path: &std::path::Path) -> FileArtifactSource {
    FileArtifactSource::new(tarball_path.parent().unwrap())
        .with_override("demo-lib", tarball_path.to_path_buf())
}

#[test]
fn concurrent_agents_share_cas_with_explicit_owners() {
    let _guard = lock_home();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));

    let tgz = pack_npm_tarball(&[
        ("package.json", br#"{"name":"demo-lib","version":"1.0.0"}"#),
        ("index.js", b"module.exports = 1;\n"),
    ]);
    let integrity = sri(&tgz);
    let tarball_dir = tmp.path().join("tarballs");
    fs::create_dir_all(&tarball_dir).unwrap();
    let tarball_path = tarball_dir.join("demo-lib-1.0.0.tgz");
    fs::write(&tarball_path, &tgz).unwrap();

    let n = 6usize;
    let mut roots = Vec::new();
    for i in 0..n {
        let p = tmp.path().join(format!("agent-{i}"));
        write_tiny_project(&p, &integrity);
        init_project(&p).unwrap();
        roots.push(p);
    }

    switch_project_with_source(&roots[0], None, &file_source(&tarball_path)).unwrap();

    let errors = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    for (i, root) in roots.iter().enumerate() {
        let root = root.clone();
        let source = file_source(&tarball_path);
        let errors = Arc::clone(&errors);
        let owner = format!("agent-{i}");
        handles.push(thread::spawn(move || {
            let opts = SwitchOptions {
                with_exec: false,
                owner: Some(owner.clone()),
            };
            match switch_project_with_source_options(&root, None, &source, &opts) {
                Ok(out) => {
                    assert_eq!(
                        out.prepare.environment.owner.as_deref(),
                        Some(owner.as_str())
                    );
                    assert!(root.join("node_modules/demo-lib").exists());
                    let status = project_status(&root).unwrap();
                    assert!(status.materialization.active_environment.is_some());
                    assert_eq!(status.environment.environments.len(), 1);
                    assert_eq!(
                        status.environment.environments[0].owner.as_deref(),
                        Some(owner.as_str())
                    );
                    let listed = env_list_entries(&root, Some(&owner)).unwrap();
                    assert_eq!(listed.len(), 1);
                    assert!(listed[0].active);
                }
                Err(e) => errors.lock().unwrap().push(format!("{owner}: {e}")),
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let errs = errors.lock().unwrap();
    assert!(errs.is_empty(), "agent failures: {errs:?}");

    let active_id = project_status(&roots[1])
        .unwrap()
        .materialization
        .active_environment
        .unwrap();
    assert!(env_remove(&roots[1], &active_id).is_err());

    let prune = env_prune(
        &roots[1],
        &EnvPruneOpts {
            owner: "agent-1".into(),
            older_than_secs: None,
            dry_run: true,
        },
    )
    .unwrap();
    assert!(prune.removed_ids.is_empty());
    assert_eq!(prune.skipped_active.as_deref(), Some(active_id.as_str()));

    assert!(env_prune(
        &roots[1],
        &EnvPruneOpts {
            owner: "  ".into(),
            older_than_secs: None,
            dry_run: true,
        },
    )
    .is_err());

    let created = env_create_with_opts(
        &roots[2],
        &EnvCreateOpts {
            label: Some("agent-label".into()),
            owner: Some("agent-2".into()),
        },
    )
    .unwrap();
    assert_eq!(created.owner.as_deref(), Some("agent-2"));
    assert_eq!(created.label.as_deref(), Some("agent-label"));

    std::env::remove_var("WEAVE_HOME");
}

#[test]
fn env_remove_and_prune_abandoned_owner_records() {
    let _guard = lock_home();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));

    let tgz = pack_npm_tarball(&[
        ("package.json", br#"{"name":"demo-lib","version":"1.0.0"}"#),
        ("index.js", b"module.exports = 1;\n"),
    ]);
    let integrity = sri(&tgz);
    let tarball_dir = tmp.path().join("tarballs");
    fs::create_dir_all(&tarball_dir).unwrap();
    let tarball_path = tarball_dir.join("demo-lib-1.0.0.tgz");
    fs::write(&tarball_path, &tgz).unwrap();

    let project = tmp.path().join("app");
    write_tiny_project(&project, &integrity);
    init_project(&project).unwrap();
    switch_project_with_source(&project, None, &file_source(&tarball_path)).unwrap();

    let store = EnvironmentStore::open(&project);
    let graph = parse_lockfile(&project.join("package-lock.json")).unwrap();
    let abandoned_id = weave_engine::EnvironmentId::derive(
        &graph,
        &PlatformIdentity {
            os: "otheros".into(),
            arch: "x86_64".into(),
        },
    );
    let abandoned = EnvironmentRecord {
        id: abandoned_id.clone(),
        graph_identity: graph.identity(),
        platform: PlatformIdentity {
            os: "otheros".into(),
            arch: "x86_64".into(),
        },
        materialization_version: "4".into(),
        lockfile_version: 3,
        package_count: 1,
        label: Some("stale".into()),
        owner: Some("agent-orphan".into()),
        created_at: Some("1".into()),
        last_activated_at: Some("1".into()),
        artifacts: Default::default(),
    };
    let active = store.active_id().unwrap().unwrap();
    assert_ne!(abandoned.id, active);
    store.save(&abandoned).unwrap();

    let removed = env_remove(&project, abandoned.id.as_str()).unwrap();
    assert_eq!(removed.owner.as_deref(), Some("agent-orphan"));

    store.save(&abandoned).unwrap();
    let prune = env_prune(
        &project,
        &EnvPruneOpts {
            owner: "agent-orphan".into(),
            older_than_secs: Some(0),
            dry_run: false,
        },
    )
    .unwrap();
    assert_eq!(prune.removed_ids, vec![abandoned.id.to_string()]);
    assert!(store.get(&abandoned.id).is_err());
    assert_eq!(store.active_id().unwrap().unwrap(), active);

    std::env::remove_var("WEAVE_HOME");
}
