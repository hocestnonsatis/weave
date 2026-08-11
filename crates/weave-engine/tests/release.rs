//! Phase 13: release hardening — recovery, config compat, hash/pack, security footguns.

use std::fs;
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

use sha2::{Digest, Sha256};
use weave_engine::{
    apply_policy_pack, doctor_project, hash_verified_artifact, init_project, load_policy_pack,
    switch_project_with_source, FileArtifactSource, HashArtifactRequest, ProjectConfig,
    SwitchOptions, WEAVE_CONFIG_VERSION,
};
use weave_fs::pack_npm_tarball;

static LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|p| p.into_inner())
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
        .args(["init", "-b", "main"])
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

fn tiny_project(tmp: &std::path::Path) -> (std::path::PathBuf, FileArtifactSource, Vec<u8>) {
    let project = tmp.join("app");
    fs::create_dir_all(&project).unwrap();
    let pkg = pack_npm_tarball(&[
        ("package.json", br#"{"name":"ms","version":"1.0.0"}"#),
        ("index.js", b"module.exports = 'ms';\n"),
    ]);
    let integrity = sri(&pkg);
    fs::write(
        project.join("package.json"),
        r#"{"name":"app","version":"1.0.0","dependencies":{"ms":"1.0.0"}}"#,
    )
    .unwrap();
    fs::write(
        project.join("package-lock.json"),
        format!(
            r#"{{
  "name": "app",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "packages": {{
    "": {{ "dependencies": {{ "ms": "1.0.0" }} }},
    "node_modules/ms": {{
      "version": "1.0.0",
      "resolved": "https://example.invalid/ms.tgz",
      "integrity": "{integrity}"
    }}
  }}
}}"#
        ),
    )
    .unwrap();
    fs::write(project.join("app.js"), "console.log(require('ms'));\n").unwrap();
    git_init(&project);
    let tarball = tmp.join("ms.tgz");
    fs::write(&tarball, &pkg).unwrap();
    let source = FileArtifactSource::new(tmp).with_override("ms", tarball);
    (project, source, pkg)
}

#[test]
fn fresh_clone_reproducible_switch_and_run() {
    let _g = lock();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
    let (project, source, _) = tiny_project(tmp.path());

    init_project(&project).unwrap();
    switch_project_with_source(&project, None, &source).unwrap();
    let out1 = Command::new("node")
        .arg("app.js")
        .current_dir(&project)
        .output()
        .unwrap();
    assert!(out1.status.success());
    assert!(String::from_utf8_lossy(&out1.stdout).contains("ms"));

    // Second switch (warm) must not break the tree.
    switch_project_with_source(&project, None, &source).unwrap();
    let out2 = Command::new("node")
        .arg("app.js")
        .current_dir(&project)
        .output()
        .unwrap();
    assert!(out2.status.success());

    let doctor = doctor_project(&project).unwrap();
    assert!(!doctor.has_errors());
    assert!(doctor
        .adoption
        .as_ref()
        .is_some_and(|a| !a.execution_config_required));

    std::env::remove_var("WEAVE_HOME");
}

#[test]
fn leftover_candidate_is_diagnosed_not_activated_as_live() {
    let _g = lock();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
    let (project, source, _) = tiny_project(tmp.path());
    init_project(&project).unwrap();
    switch_project_with_source(&project, None, &source).unwrap();

    let candidate = project.join(".weave/candidate");
    fs::create_dir_all(candidate.join("node_modules/orphan")).unwrap();
    fs::write(
        candidate.join("node_modules/orphan/package.json"),
        r#"{"name":"orphan"}"#,
    )
    .unwrap();

    let doctor = doctor_project(&project).unwrap();
    assert!(doctor
        .findings
        .iter()
        .any(|f| f.check == "candidate" && f.message.contains("leftover")));
    // Live tree still works.
    assert!(project.join("node_modules/ms/index.js").is_file());
    assert!(!project.join("node_modules/orphan").exists());

    std::env::remove_var("WEAVE_HOME");
}

#[test]
fn corrupt_config_fails_closed() {
    let _g = lock();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
    let (project, _source, _) = tiny_project(tmp.path());
    init_project(&project).unwrap();
    fs::write(
        project.join(".weave/config.toml"),
        "version = 1\nstore_path = \"/tmp\"\nmaterialization_version = \"x\"\n\
         [execution]\nenabled = false\nprofile = \"open\"\n",
    )
    .unwrap();
    let err = ProjectConfig::load(&project).unwrap_err();
    assert!(err.to_string().contains("open") || err.to_string().contains("rejected"));

    std::env::remove_var("WEAVE_HOME");
}

#[test]
fn with_exec_without_enable_leaves_tree_untouched_on_failure() {
    let _g = lock();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
    let (project, source, _) = tiny_project(tmp.path());
    init_project(&project).unwrap();
    switch_project_with_source(&project, None, &source).unwrap();
    let before = fs::read_to_string(project.join("node_modules/ms/index.js")).unwrap();

    let err = weave_engine::switch_project_with_source_options(
        &project,
        None,
        &source,
        &SwitchOptions { with_exec: true },
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("enabled") || err.to_string().contains("with-exec"),
        "{err}"
    );
    let after = fs::read_to_string(project.join("node_modules/ms/index.js")).unwrap();
    assert_eq!(before, after);

    std::env::remove_var("WEAVE_HOME");
}

#[test]
fn hash_artifact_matches_policy_pack_fixture() {
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/policy-packs/example-native.node");
    let report = hash_verified_artifact(&HashArtifactRequest {
        path: fixture,
        package: "example-native".into(),
        output: "prebuilds/linux-x64/addon.node".into(),
        url: Some("https://cdn.example.com/example-native/linux-x64-137.node".into()),
        node_abi: Some("137".into()),
        os: Some("linux".into()),
        cpu: Some("x64".into()),
    })
    .unwrap();
    let pack = load_policy_pack(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/policy-packs/example-demo.toml"),
    )
    .unwrap();
    assert_eq!(report.integrity, pack.prebuild.fetches[0].integrity);
    assert!(!report.toml_fragment.contains("enabled = true"));
}

#[test]
fn apply_policy_pack_never_enables_execution() {
    let mut cfg = ProjectConfig::new("/tmp/store");
    assert_eq!(cfg.version, WEAVE_CONFIG_VERSION);
    let pack = load_policy_pack(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/policy-packs/example-demo.toml"),
    )
    .unwrap();
    let report = apply_policy_pack(&mut cfg, &pack);
    assert!(!cfg.execution.enabled);
    assert_eq!(cfg.execution.profile, "offline");
    assert!(report.enabled_unchanged);
    assert!(report.fetches_added >= 1);
}

#[test]
fn refuse_live_node_modules_input_for_exec() {
    let _g = lock();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
    let (project, source, _) = tiny_project(tmp.path());
    init_project(&project).unwrap();
    switch_project_with_source(&project, None, &source).unwrap();

    // Force-enable in config but still refuse live node_modules as exec input.
    let mut cfg = ProjectConfig::load(&project).unwrap();
    cfg.execution.enabled = true;
    cfg.execution.allow_packages = vec!["ms".into()];
    cfg.execution
        .declared_outputs
        .insert("ms".into(), vec!["index.js".into()]);
    fs::write(
        project.join(".weave/config.toml"),
        cfg.to_toml_string().unwrap(),
    )
    .unwrap();

    let live = project.join("node_modules/ms");
    let err = weave_engine::exec_run_sandboxed(&weave_engine::ExecRunRequest {
        project_root: project.clone(),
        package: "ms".into(),
        input_package_dir: live,
        script_rel: std::path::PathBuf::from("index.js"),
    })
    .unwrap_err();
    assert!(
        err.to_string().contains("node_modules") || err.to_string().contains("live"),
        "{err}"
    );

    std::env::remove_var("WEAVE_HOME");
}
