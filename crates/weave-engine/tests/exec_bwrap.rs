//! Bubblewrap offline execution integration (gated).
//!
//! Enable with: `WEAVE_EXEC_TESTS=1 cargo test -p weave-engine --test exec_bwrap -- --ignored`

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use weave_engine::{
    bwrap_available, exec_run_sandboxed, ExecRunRequest, ExecutionConfig, ProjectConfig,
};
use weave_store::ContentStore;

static LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

fn gated() -> bool {
    matches!(
        std::env::var("WEAVE_EXEC_TESTS").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

fn fixture_pkg() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/exec-offline-gen")
}

fn write_enabled_project(root: &std::path::Path) {
    fs::create_dir_all(root.join(".weave")).unwrap();
    let store = root.join("store");
    fs::create_dir_all(&store).unwrap();
    let mut declared = BTreeMap::new();
    declared.insert("exec-gen".into(), vec!["generated/hello.txt".into()]);
    let cfg = ProjectConfig {
        version: 1,
        store_path: store.display().to_string(),
        materialization_version: "4".into(),
        execution: ExecutionConfig {
            enabled: true,
            profile: "offline".into(),
            allow_packages: vec!["exec-gen".into()],
            allow_scripts: vec!["install".into()],
            declared_outputs: declared,
            allow_weak_sandbox: false,
            prebuild: Default::default(),
        },
    };
    fs::write(
        root.join(".weave/config.toml"),
        toml::to_string_pretty(&cfg).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "set WEAVE_EXEC_TESTS=1"]
fn bwrap_offline_fixture_seals_declared_output() {
    if !gated() {
        eprintln!("skip: WEAVE_EXEC_TESTS not set");
        return;
    }
    assert!(
        bwrap_available(),
        "bwrap must be installed for WEAVE_EXEC_TESTS"
    );
    let _g = lock();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
    write_enabled_project(tmp.path());

    // Copy fixture out of the repo tree into an ephemeral input (not node_modules).
    let input = tmp.path().join("input/exec-gen");
    copy_tree(&fixture_pkg(), &input);

    let report = exec_run_sandboxed(&ExecRunRequest {
        project_root: tmp.path().to_path_buf(),
        package: "exec-gen".into(),
        input_package_dir: input,
        script_rel: PathBuf::from("scripts/install.js"),
    })
    .expect("sandboxed run");

    assert_eq!(report.seal.sealed_paths, vec!["generated/hello.txt"]);
    let store = ContentStore::open(tmp.path().join("store")).unwrap();
    assert!(store.contains(&report.seal.output_artifact_id));
    assert!(!report.seal.cache_key.is_empty());

    // Work tree should contain the generated file; project node_modules untouched.
    assert!(!tmp.path().join("node_modules").exists());
    let hello = report.work_root.join("work/generated/hello.txt");
    assert_eq!(fs::read_to_string(hello).unwrap(), "weave-exec-ok\n");

    std::env::remove_var("WEAVE_HOME");
}

fn copy_tree(src: &std::path::Path, dst: &std::path::Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to);
        } else {
            fs::copy(&from, &to).unwrap();
        }
    }
}
