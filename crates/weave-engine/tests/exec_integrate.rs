//! Phase 8: sealed execution output integration with switch/materialize.
//!
//! Real Bubblewrap cases are gated with `WEAVE_EXEC_TESTS=1`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

use sha2::{Digest, Sha256};
use weave_engine::{
    apply_sealed_outputs, build_exec_identity, bwrap_available,
    ensure_package_outputs_on_candidate, init_project, persist_exec_cache, seal_declared_outputs,
    switch_project_with_source, switch_project_with_source_options, ExecCacheRecord,
    ExecutionConfig, FileArtifactSource, ProjectConfig, SwitchOptions,
};
use weave_store::{hash_bytes, ContentStore};

static LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

fn gated_bwrap() -> bool {
    matches!(
        std::env::var("WEAVE_EXEC_TESTS").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    ) && bwrap_available()
}

fn fixture_pkg() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/exec-offline-gen")
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

fn git_init(dir: &Path) {
    assert!(Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(dir)
        .status()
        .unwrap()
        .success());
    let _ = Command::new("git")
        .args(["config", "user.email", "weave@example.com"])
        .current_dir(dir)
        .status();
    let _ = Command::new("git")
        .args(["config", "user.name", "Weave Test"])
        .current_dir(dir)
        .status();
}

fn commit_all(dir: &Path, msg: &str) {
    assert!(Command::new("git")
        .args(["add", "."])
        .current_dir(dir)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["commit", "-m", msg])
        .current_dir(dir)
        .status()
        .unwrap()
        .success());
}

struct ProjectHarness {
    _tmp: tempfile::TempDir,
    project: PathBuf,
    tarball_dir: PathBuf,
    source: FileArtifactSource,
}

fn setup_exec_gen_project(execution_enabled: bool) -> ProjectHarness {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
    let project = tmp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    git_init(&project);

    // Pack fixture without generated/ (script creates it).
    let fixture = fixture_pkg();
    let pack_dir = tmp.path().join("pack/exec-gen");
    copy_tree(&fixture, &pack_dir);
    let _ = fs::remove_dir_all(pack_dir.join("generated"));

    let tgz = weave_fs::pack_directory_as_npm_tarball(&pack_dir).unwrap();
    let digest = Sha256::digest(&tgz);
    let integrity = format!("sha256-{}", b64(&digest));

    fs::write(
        project.join("package.json"),
        r#"{"name":"demo","version":"1.0.0","dependencies":{"exec-gen":"1.0.0"}}"#,
    )
    .unwrap();
    fs::write(
        project.join("package-lock.json"),
        format!(
            r#"{{
  "name": "demo",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "packages": {{
    "": {{
      "name": "demo",
      "version": "1.0.0",
      "dependencies": {{ "exec-gen": "1.0.0" }}
    }},
    "node_modules/exec-gen": {{
      "version": "1.0.0",
      "resolved": "https://example.invalid/exec-gen/-/exec-gen-1.0.0.tgz",
      "integrity": "{integrity}"
    }}
  }}
}}"#
        ),
    )
    .unwrap();
    fs::write(project.join("README"), "x").unwrap();
    commit_all(&project, "init");

    init_project(&project).unwrap();

    let mut declared = BTreeMap::new();
    declared.insert("exec-gen".into(), vec!["generated/hello.txt".into()]);
    let mut cfg = ProjectConfig::load(&project).unwrap();
    cfg.execution = ExecutionConfig {
        enabled: execution_enabled,
        profile: "offline".into(),
        allow_packages: vec!["exec-gen".into()],
        allow_scripts: vec!["install".into()],
        declared_outputs: declared,
        allow_weak_sandbox: false,
        prebuild: Default::default(),
    };
    fs::write(
        project.join(".weave/config.toml"),
        toml::to_string_pretty(&cfg).unwrap(),
    )
    .unwrap();

    let tarball_dir = tmp.path().join("tarballs");
    fs::create_dir_all(&tarball_dir).unwrap();
    let tarball_path = tarball_dir.join("exec-gen.tgz");
    fs::write(&tarball_path, &tgz).unwrap();
    let source =
        FileArtifactSource::new(&tarball_dir).with_override("exec-gen", tarball_path.clone());

    ProjectHarness {
        _tmp: tmp,
        project,
        tarball_dir,
        source,
    }
}

fn copy_tree(src: &Path, dst: &Path) {
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

fn hello_path(project: &Path) -> PathBuf {
    project.join("node_modules/exec-gen/generated/hello.txt")
}

#[test]
fn plain_switch_stays_execution_free_even_when_config_enabled() {
    let _g = lock();
    let h = setup_exec_gen_project(true);
    let outcome = switch_project_with_source(&h.project, None, &h.source).unwrap();
    assert_eq!(outcome.prepare.execution.packages_considered, 0);
    assert_eq!(outcome.prepare.execution.executed, 0);
    assert!(!hello_path(&h.project).exists());
    assert!(h
        .project
        .join("node_modules/exec-gen/package.json")
        .is_file());
    std::env::remove_var("WEAVE_HOME");
}

#[test]
fn with_exec_rejected_when_config_disabled() {
    let _g = lock();
    let h = setup_exec_gen_project(false);
    let err = switch_project_with_source_options(
        &h.project,
        None,
        &h.source,
        &SwitchOptions { with_exec: true },
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("--with-exec") || err.to_string().contains("enabled"),
        "{err}"
    );
    assert!(!h.project.join("node_modules").exists());
    std::env::remove_var("WEAVE_HOME");
}

#[test]
fn cache_hit_applies_declared_output_without_bwrap() {
    let _g = lock();
    let h = setup_exec_gen_project(true);

    // First: plain materialize/switch so package exists in store + we can build identity.
    let first = switch_project_with_source(&h.project, None, &h.source).unwrap();
    assert!(!hello_path(&h.project).exists());
    assert_eq!(first.prepare.execution.executed, 0);

    // Plant a sealed artifact + cache index matching the pristine package digest.
    let pkg = h.project.join("node_modules/exec-gen");
    // Use a copy under candidate-like path for identity (same content as pristine).
    let pristine = h._tmp.path().join("pristine-pkg");
    copy_tree(&pkg, &pristine);
    // Remove any accidental generated/
    let _ = fs::remove_dir_all(pristine.join("generated"));

    let cfg = ProjectConfig::load(&h.project).unwrap();
    let identity = build_exec_identity(&cfg.execution, "exec-gen", &pristine).unwrap();
    let store = ContentStore::open(PathBuf::from(&cfg.store_path)).unwrap();

    let stage = h._tmp.path().join("seal-stage");
    fs::create_dir_all(stage.join("generated")).unwrap();
    fs::write(stage.join("generated/hello.txt"), b"weave-exec-ok\n").unwrap();
    let seal = seal_declared_outputs(&store, &stage, &["generated/hello.txt".into()]).unwrap();

    persist_exec_cache(&ExecCacheRecord {
        package: "exec-gen".into(),
        output_artifact_id: seal.output_artifact_id.to_string(),
        cache_key: identity.cache_key(),
        sealed_paths: vec!["generated/hello.txt".into()],
        node_abi: identity.node_abi.clone(),
        os: identity.os.clone(),
        cpu: identity.cpu.clone(),
        profile: identity.profile.clone(),
    })
    .unwrap();

    // Break bwrap — cache hit must not need it.
    let prev = std::env::var_os("WEAVE_BWRAP_PATH");
    std::env::set_var("WEAVE_BWRAP_PATH", "/nonexistent/bwrap-binary");

    let again = switch_project_with_source_options(
        &h.project,
        None,
        &h.source,
        &SwitchOptions { with_exec: true },
    )
    .expect("cache hit switch");
    assert_eq!(again.prepare.execution.packages_considered, 1);
    assert_eq!(again.prepare.execution.cache_hits, 1);
    assert_eq!(again.prepare.execution.executed, 0);
    assert_eq!(again.prepare.execution.applied, 1);
    assert_eq!(
        fs::read_to_string(hello_path(&h.project)).unwrap(),
        "weave-exec-ok\n"
    );

    match prev {
        Some(v) => std::env::set_var("WEAVE_BWRAP_PATH", v),
        None => std::env::remove_var("WEAVE_BWRAP_PATH"),
    }
    std::env::remove_var("WEAVE_HOME");
}

#[test]
fn undeclared_output_never_enters_package_dir() {
    let _g = lock();
    let tmp = tempfile::tempdir().unwrap();
    let store = ContentStore::open(tmp.path().join("store")).unwrap();
    let stage = tmp.path().join("stage");
    fs::create_dir_all(stage.join("generated")).unwrap();
    fs::write(stage.join("generated/hello.txt"), b"ok\n").unwrap();
    fs::write(stage.join("undeclared.bin"), b"evil").unwrap();
    let tgz = weave_fs::pack_directory_as_npm_tarball(&stage).unwrap();
    let id = hash_bytes(&tgz);
    store.put(&tgz, Some(&id)).unwrap();

    let pkg = tmp.path().join("pkg");
    fs::create_dir_all(&pkg).unwrap();
    let err = apply_sealed_outputs(&store, &id, &pkg, &["generated/hello.txt".into()]).unwrap_err();
    assert!(err.to_string().contains("undeclared"), "{err}");
    assert!(!pkg.join("undeclared.bin").exists());
}

#[test]
fn platform_abi_cache_mismatch_does_not_hit() {
    let _g = lock();
    let h = setup_exec_gen_project(true);
    switch_project_with_source(&h.project, None, &h.source).unwrap();

    let pkg = h.project.join("node_modules/exec-gen");
    let cfg = ProjectConfig::load(&h.project).unwrap();
    let identity = build_exec_identity(&cfg.execution, "exec-gen", &pkg).unwrap();
    let store = ContentStore::open(PathBuf::from(&cfg.store_path)).unwrap();

    let stage = h._tmp.path().join("seal-stage");
    fs::create_dir_all(stage.join("generated")).unwrap();
    fs::write(stage.join("generated/hello.txt"), b"weave-exec-ok\n").unwrap();
    let seal = seal_declared_outputs(&store, &stage, &["generated/hello.txt".into()]).unwrap();

    // Plant under the correct cache_key filename but with wrong ABI in the record.
    // verify_exec_cache_hit must reject; ensure then needs execute (or fails without bwrap).
    persist_exec_cache(&ExecCacheRecord {
        package: "exec-gen".into(),
        output_artifact_id: seal.output_artifact_id.to_string(),
        cache_key: identity.cache_key(),
        sealed_paths: vec!["generated/hello.txt".into()],
        node_abi: "99999".into(),
        os: identity.os.clone(),
        cpu: identity.cpu.clone(),
        profile: identity.profile.clone(),
    })
    .unwrap();

    let prev = std::env::var_os("WEAVE_BWRAP_PATH");
    std::env::set_var("WEAVE_BWRAP_PATH", "/nonexistent/bwrap-binary");

    // Direct ensure: stale ABI record must not apply; without bwrap → error.
    let cand = h.project.join(".weave/candidate/node_modules/exec-gen");
    // Build a candidate-like copy for ensure (not live nm).
    let isolated = h._tmp.path().join("isolated/exec-gen");
    copy_tree(&pkg, &isolated);
    let _ = fs::remove_dir_all(isolated.join("generated"));

    let err =
        ensure_package_outputs_on_candidate(&h.project, "exec-gen", &isolated, true).unwrap_err();
    assert!(
        err.to_string().contains("sandbox unavailable") || err.to_string().contains("cache"),
        "{err}"
    );
    assert!(!isolated.join("generated/hello.txt").exists());
    let _ = cand;

    match prev {
        Some(v) => std::env::set_var("WEAVE_BWRAP_PATH", v),
        None => std::env::remove_var("WEAVE_BWRAP_PATH"),
    }
    std::env::remove_var("WEAVE_HOME");
}

#[test]
fn failed_execution_leaves_active_untouched() {
    let _g = lock();
    let h = setup_exec_gen_project(true);

    // Activate a known-good tree first (no exec).
    switch_project_with_source(&h.project, None, &h.source).unwrap();
    assert!(h
        .project
        .join("node_modules/exec-gen/package.json")
        .is_file());
    assert!(!hello_path(&h.project).exists());
    let marker = h.project.join("node_modules/.weave-active-marker");
    fs::write(&marker, b"keep-me").unwrap();

    // Force an execution miss with no usable sandbox → prepare fails before activate.
    let prev = std::env::var_os("WEAVE_BWRAP_PATH");
    std::env::set_var("WEAVE_BWRAP_PATH", "/nonexistent/bwrap-binary");

    let err = switch_project_with_source_options(
        &h.project,
        None,
        &h.source,
        &SwitchOptions { with_exec: true },
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("sandbox unavailable")
            || err.to_string().contains("failed")
            || err.to_string().contains("execution"),
        "{err}"
    );

    // Active tree untouched.
    assert_eq!(fs::read_to_string(&marker).unwrap(), "keep-me");
    assert!(!hello_path(&h.project).exists());
    assert!(h
        .project
        .join("node_modules/exec-gen/package.json")
        .is_file());

    match prev {
        Some(v) => std::env::set_var("WEAVE_BWRAP_PATH", v),
        None => std::env::remove_var("WEAVE_BWRAP_PATH"),
    }
    std::env::remove_var("WEAVE_HOME");
}

#[test]
#[ignore = "set WEAVE_EXEC_TESTS=1"]
fn first_execution_and_declared_output_activation() {
    if !gated_bwrap() {
        eprintln!("skip: WEAVE_EXEC_TESTS / bwrap unavailable");
        return;
    }
    let _g = lock();
    let h = setup_exec_gen_project(true);

    let first = switch_project_with_source_options(
        &h.project,
        None,
        &h.source,
        &SwitchOptions { with_exec: true },
    )
    .expect("first with-exec");
    assert_eq!(first.prepare.execution.executed, 1);
    assert_eq!(first.prepare.execution.cache_hits, 0);
    assert_eq!(
        fs::read_to_string(hello_path(&h.project)).unwrap(),
        "weave-exec-ok\n"
    );

    // Second switch: cache hit, no re-exec.
    let prev = std::env::var_os("WEAVE_BWRAP_PATH");
    std::env::set_var("WEAVE_BWRAP_PATH", "/nonexistent/bwrap-binary");
    let second = switch_project_with_source_options(
        &h.project,
        None,
        &h.source,
        &SwitchOptions { with_exec: true },
    )
    .expect("cache hit with-exec");
    assert_eq!(second.prepare.execution.cache_hits, 1);
    assert_eq!(second.prepare.execution.executed, 0);
    assert_eq!(
        fs::read_to_string(hello_path(&h.project)).unwrap(),
        "weave-exec-ok\n"
    );
    match prev {
        Some(v) => std::env::set_var("WEAVE_BWRAP_PATH", v),
        None => std::env::remove_var("WEAVE_BWRAP_PATH"),
    }
    std::env::remove_var("WEAVE_HOME");
}

#[test]
#[ignore = "set WEAVE_EXEC_TESTS=1"]
fn failed_script_leaves_active_untouched() {
    if !gated_bwrap() {
        eprintln!("skip: WEAVE_EXEC_TESTS / bwrap unavailable");
        return;
    }
    let _g = lock();
    let h = setup_exec_gen_project(true);

    switch_project_with_source(&h.project, None, &h.source).unwrap();
    let marker = h.project.join("node_modules/.weave-active-marker");
    fs::write(&marker, b"keep-me").unwrap();

    let pack_dir = h._tmp.path().join("pack-fail/exec-gen");
    copy_tree(&fixture_pkg(), &pack_dir);
    let _ = fs::remove_dir_all(pack_dir.join("generated"));
    fs::write(
        pack_dir.join("scripts/install.js"),
        b"console.error('intentional fail'); process.exit(1);\n",
    )
    .unwrap();
    let tgz = weave_fs::pack_directory_as_npm_tarball(&pack_dir).unwrap();
    let digest = Sha256::digest(&tgz);
    let integrity = format!("sha256-{}", b64(&digest));
    let lock = fs::read_to_string(h.project.join("package-lock.json"))
        .unwrap()
        .lines()
        .map(|line| {
            if line.contains("\"integrity\"") {
                format!("      \"integrity\": \"{integrity}\"")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(h.project.join("package-lock.json"), lock).unwrap();
    let tarball_path = h.tarball_dir.join("exec-gen.tgz");
    fs::write(&tarball_path, &tgz).unwrap();
    let source = FileArtifactSource::new(&h.tarball_dir).with_override("exec-gen", tarball_path);

    let err = switch_project_with_source_options(
        &h.project,
        None,
        &source,
        &SwitchOptions { with_exec: true },
    )
    .unwrap_err();
    assert!(err.to_string().contains("failed"), "{err}");
    assert_eq!(fs::read_to_string(&marker).unwrap(), "keep-me");
    assert!(!hello_path(&h.project).exists());

    std::env::remove_var("WEAVE_HOME");
}
