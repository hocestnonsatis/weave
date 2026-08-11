//! Phase 9: policy discovery must not execute or auto-approve.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

use sha2::{Digest, Sha256};
use weave_engine::{
    discover_package_dir, exec_plan_for_project, init_project, merge_suggestion_into_config,
    suggest_execution_policy, switch_project_with_source, FileArtifactSource, PolicyReviewStatus,
    SwitchOptions,
};

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

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/policy-discovery")
        .join(name)
}

#[test]
fn project_plan_distinguishes_discovered_from_allowed() {
    let _g = lock();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
    let project = tmp.path().join("project");
    fs::create_dir_all(&project).unwrap();

    assert!(Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&project)
        .status()
        .unwrap()
        .success());
    let _ = Command::new("git")
        .args(["config", "user.email", "weave@example.com"])
        .current_dir(&project)
        .status();
    let _ = Command::new("git")
        .args(["config", "user.name", "Weave Test"])
        .current_dir(&project)
        .status();

    let native = fixture("native-binding");
    let tgz = weave_fs::pack_directory_as_npm_tarball(&native).unwrap();
    let integrity = format!("sha256-{}", b64(&Sha256::digest(&tgz)));

    fs::write(
        project.join("package.json"),
        r#"{"name":"demo","version":"1.0.0","dependencies":{"demo-native":"1.0.0"}}"#,
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
    "": {{ "name": "demo", "version": "1.0.0", "dependencies": {{ "demo-native": "1.0.0" }} }},
    "node_modules/demo-native": {{
      "version": "1.0.0",
      "resolved": "https://example.invalid/demo-native-1.0.0.tgz",
      "integrity": "{integrity}",
      "hasInstallScript": true
    }}
  }}
}}"#
        ),
    )
    .unwrap();
    assert!(Command::new("git")
        .args(["add", "."])
        .current_dir(&project)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["commit", "-m", "init"])
        .current_dir(&project)
        .status()
        .unwrap()
        .success());

    init_project(&project).unwrap();
    let tarball_dir = tmp.path().join("tarballs");
    fs::create_dir_all(&tarball_dir).unwrap();
    let tarball_path = tarball_dir.join("demo-native.tgz");
    fs::write(&tarball_path, &tgz).unwrap();
    let source = FileArtifactSource::new(&tarball_dir).with_override("demo-native", tarball_path);

    // Plain switch — execution-free.
    let outcome = switch_project_with_source(&project, None, &source).unwrap();
    assert_eq!(outcome.prepare.execution.executed, 0);
    assert!(!project
        .join("node_modules/demo-native/build/Release/demo_native.node")
        .exists());

    let (plan, cfg) = exec_plan_for_project(&project).unwrap();
    assert!(!plan.executed);
    assert!(!cfg.execution.enabled);
    let entry = plan
        .entries
        .iter()
        .find(|e| e.name.as_deref() == Some("demo-native"))
        .expect("demo-native in plan");
    assert!(entry.needs_execution);
    assert!(!entry.would_execute);
    assert!(entry.metadata_loaded);
    assert!(!entry.discovered_output_candidates.is_empty());
    assert!(entry.allowed_outputs.is_empty());
    assert_eq!(entry.policy, PolicyReviewStatus::NeedsReview);

    let discovery = discover_package_dir(&project.join("node_modules/demo-native")).unwrap();
    let suggestion = suggest_execution_policy(&[discovery], &cfg.execution);
    assert!(!suggestion.enabled_suggestion);
    assert!(suggestion.allow_packages.iter().any(|p| p == "demo-native"));

    let mut merged = cfg.execution.clone();
    merge_suggestion_into_config(&mut merged, &suggestion);
    assert!(!merged.enabled);

    // Dual gate still required — with_exec without enabled fails.
    let err = weave_engine::switch_project_with_source_options(
        &project,
        None,
        &source,
        &SwitchOptions { with_exec: true },
    )
    .unwrap_err();
    assert!(err.to_string().contains("--with-exec") || err.to_string().contains("enabled"));

    std::env::remove_var("WEAVE_HOME");
}

#[test]
fn discovery_never_executes_fixture_scripts() {
    // Guarantee: reading fixtures does not create generated outputs.
    let esbuild = fixture("esbuild-like");
    let before = esbuild.join("bin/esbuild");
    let _ = fs::remove_file(&before);
    let d = discover_package_dir(&esbuild).unwrap();
    assert!(d.needs_execution);
    assert!(!before.exists(), "discovery must not run install.js");
}
