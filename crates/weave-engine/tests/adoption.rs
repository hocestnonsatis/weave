//! Phase 12: real-world adoption workflows (offline fixtures).
//!
//! Covers clone/init → prepare → switch → run for extraction-only projects,
//! and clear failure / partial diagnostics for native + unsafe packages.
//! Never invents SRI, never grants script network, never executes install scripts
//! unless dual-gated (not used here for incomplete natives).

use std::fs;
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

use sha2::{Digest, Sha256};
use weave_engine::{
    doctor_project, exec_plan_with_adoption, init_project, switch_project_with_source,
    AdoptionVerdict, DoctorSeverity, FileArtifactSource, SwitchOptions,
};
use weave_fs::pack_directory_as_npm_tarball;

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

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/adoption")
        .join(name)
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

#[test]
fn extraction_only_init_switch_run_needs_no_execution_config() {
    let _g = lock();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
    let project = tmp.path().join("app");
    fs::create_dir_all(&project).unwrap();

    let pkg_dir = fixture("extraction-only");
    let tgz = pack_directory_as_npm_tarball(&pkg_dir).unwrap();
    let integrity = sri(&tgz);

    fs::write(
        project.join("package.json"),
        r#"{"name":"extraction-app","version":"1.0.0","dependencies":{"left-pad-like":"1.0.0"}}"#,
    )
    .unwrap();
    fs::write(
        project.join("package-lock.json"),
        format!(
            r#"{{
  "name": "extraction-app",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "packages": {{
    "": {{
      "name": "extraction-app",
      "version": "1.0.0",
      "dependencies": {{ "left-pad-like": "1.0.0" }}
    }},
    "node_modules/left-pad-like": {{
      "version": "1.0.0",
      "resolved": "https://example.invalid/left-pad-like.tgz",
      "integrity": "{integrity}"
    }}
  }}
}}"#
        ),
    )
    .unwrap();
    fs::write(
        project.join("app.js"),
        r#"
const pad = require('left-pad-like');
if (typeof pad !== 'function') throw new Error('pad missing');
console.log('extraction-ok');
"#,
    )
    .unwrap();
    git_init(&project);

    let tarball = tmp.path().join("left-pad-like.tgz");
    fs::write(&tarball, &tgz).unwrap();
    let source = FileArtifactSource::new(tmp.path()).with_override("left-pad-like", tarball);

    init_project(&project).unwrap();
    // No [execution] block — defaults stay disabled.
    switch_project_with_source(&project, None, &source).unwrap();

    let out = Command::new("node")
        .arg("app.js")
        .current_dir(&project)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("extraction-ok"));

    let doctor = doctor_project(&project).unwrap();
    assert!(!doctor.has_errors());
    let adoption = doctor.adoption.expect("adoption");
    assert_eq!(adoption.verdict, AdoptionVerdict::ExtractionReady);
    assert!(!adoption.execution_config_required);

    let (_plan, cfg, adoption2) = exec_plan_with_adoption(&project).unwrap();
    assert!(!cfg.execution.enabled);
    assert_eq!(adoption2.verdict, AdoptionVerdict::ExtractionReady);

    // Plain switch with --with-exec refused when disabled (dual gate).
    let err = weave_engine::switch_project_with_source_options(
        &project,
        None,
        &source,
        &SwitchOptions {
            with_exec: true,
            owner: None,
        },
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("enabled") || err.to_string().contains("with-exec"),
        "{err}"
    );

    std::env::remove_var("WEAVE_HOME");
}

#[test]
fn native_incomplete_switch_succeeds_but_adoption_is_partial() {
    let _g = lock();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
    let project = tmp.path().join("native-app");
    fs::create_dir_all(&project).unwrap();

    let pkg_dir = fixture("native-incomplete");
    let tgz = pack_directory_as_npm_tarball(&pkg_dir).unwrap();
    let integrity = sri(&tgz);

    fs::write(
        project.join("package.json"),
        r#"{"name":"native-app","version":"1.0.0","dependencies":{"demo-native-incomplete":"1.0.0"}}"#,
    )
    .unwrap();
    fs::write(
        project.join("package-lock.json"),
        format!(
            r#"{{
  "name": "native-app",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "packages": {{
    "": {{
      "dependencies": {{ "demo-native-incomplete": "1.0.0" }}
    }},
    "node_modules/demo-native-incomplete": {{
      "version": "1.0.0",
      "resolved": "https://example.invalid/demo-native-incomplete.tgz",
      "integrity": "{integrity}",
      "hasInstallScript": true
    }}
  }}
}}"#
        ),
    )
    .unwrap();
    git_init(&project);

    let tarball = tmp.path().join("demo-native-incomplete.tgz");
    fs::write(&tarball, &tgz).unwrap();
    let source =
        FileArtifactSource::new(tmp.path()).with_override("demo-native-incomplete", tarball);

    init_project(&project).unwrap();
    // Extraction still works — incompleteness is diagnosed, not mysterious.
    switch_project_with_source(&project, None, &source).unwrap();
    assert!(project
        .join("node_modules/demo-native-incomplete/package.json")
        .is_file());
    assert!(!project
        .join("node_modules/demo-native-incomplete")
        .join("addon.node")
        .exists());

    let doctor = doctor_project(&project).unwrap();
    let adoption = doctor.adoption.expect("adoption");
    assert_eq!(adoption.verdict, AdoptionVerdict::PartialNeedsPolicy);
    assert!(adoption.execution_config_required);
    assert!(
        doctor.findings.iter().any(|f| {
            f.check == "adoption" && f.severity == DoctorSeverity::Warn
                || f.check == "lifecycle-scripts"
        }),
        "{:?}",
        doctor.findings
    );

    let (plan, _cfg, adoption2) = exec_plan_with_adoption(&project).unwrap();
    assert!(plan.needs_execution_count >= 1);
    assert_eq!(adoption2.verdict, AdoptionVerdict::PartialNeedsPolicy);
    assert!(
        adoption2.gaps.iter().any(|g| {
            g.package.contains("demo-native")
                && (g.user_must.contains("SRI")
                    || g.user_must.contains("prebuild")
                    || g.user_must.contains("suggest"))
        }),
        "{:?}",
        adoption2.gaps
    );
    // Must not invent fixes.
    assert!(adoption2
        .next_actions
        .iter()
        .any(|a| a.step.contains("exec plan") || a.why.contains("SRI") || a.step.contains("SRI")));

    std::env::remove_var("WEAVE_HOME");
}

#[test]
fn unsafe_lifecycle_never_suggested_and_surfaces_clearly() {
    let _g = lock();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
    let project = tmp.path().join("bad");
    fs::create_dir_all(&project).unwrap();

    let pkg_dir = fixture("unsafe-lifecycle");
    let tgz = pack_directory_as_npm_tarball(&pkg_dir).unwrap();
    let integrity = sri(&tgz);

    fs::write(
        project.join("package.json"),
        r#"{"name":"bad","version":"1.0.0","dependencies":{"unsafe-curl-pkg":"1.0.0"}}"#,
    )
    .unwrap();
    fs::write(
        project.join("package-lock.json"),
        format!(
            r#"{{
  "name": "bad",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "packages": {{
    "": {{ "dependencies": {{ "unsafe-curl-pkg": "1.0.0" }} }},
    "node_modules/unsafe-curl-pkg": {{
      "version": "1.0.0",
      "resolved": "https://example.invalid/unsafe.tgz",
      "integrity": "{integrity}",
      "hasInstallScript": true
    }}
  }}
}}"#
        ),
    )
    .unwrap();
    git_init(&project);

    let tarball = tmp.path().join("unsafe.tgz");
    fs::write(&tarball, &tgz).unwrap();
    let source = FileArtifactSource::new(tmp.path()).with_override("unsafe-curl-pkg", tarball);

    init_project(&project).unwrap();
    switch_project_with_source(&project, None, &source).unwrap();

    let (plan, cfg, adoption) = exec_plan_with_adoption(&project).unwrap();
    let entry = plan
        .entries
        .iter()
        .find(|e| e.name.as_deref() == Some("unsafe-curl-pkg"))
        .expect("entry");
    assert!(
        matches!(entry.class, weave_engine::ExecNeedClass::UnsupportedUnsafe)
            || entry.discovered_scripts.iter().any(|s| s.unsafe_body)
            || adoption.blocked_count >= 1
            || adoption.gaps.iter().any(|g| g.package == "unsafe-curl-pkg"),
        "unsafe package must surface clearly: class={:?} adoption={:?}",
        entry.class,
        adoption
    );

    let discoveries: Vec<_> = plan
        .entries
        .iter()
        .filter_map(|e| {
            weave_engine::resolve_package_dir_for_discovery(
                &project,
                &e.package_key,
                e.name.as_deref(),
            )
            .and_then(|d| weave_engine::discover_package_dir(&d).ok())
        })
        .collect();
    let suggestion = weave_engine::suggest_execution_policy(&discoveries, &cfg.execution);
    assert!(
        !suggestion
            .allow_packages
            .iter()
            .any(|p| p == "unsafe-curl-pkg"),
        "unsafe must never be auto-suggested: {:?}",
        suggestion.allow_packages
    );
    assert!(suggestion
        .blocked_packages
        .iter()
        .any(|b| b.name == "unsafe-curl-pkg"));

    std::env::remove_var("WEAVE_HOME");
}

#[test]
fn missing_required_peer_blocks_adoption_and_switch() {
    let _g = lock();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
    let project = tmp.path().join("peers");
    fs::create_dir_all(&project).unwrap();

    let ui = weave_fs::pack_npm_tarball(&[
        (
            "package.json",
            br#"{"name":"ui-kit","version":"1.0.0","peerDependencies":{"react":"^18.0.0"}}"#,
        ),
        ("index.js", b"module.exports = 1;\n"),
    ]);
    let i_ui = sri(&ui);
    fs::write(
        project.join("package.json"),
        r#"{"name":"bad-peer","version":"1.0.0"}"#,
    )
    .unwrap();
    fs::write(
        project.join("package-lock.json"),
        format!(
            r#"{{
  "name": "bad-peer",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "packages": {{
    "": {{ "dependencies": {{ "ui-kit": "1.0.0" }} }},
    "node_modules/ui-kit": {{
      "version": "1.0.0",
      "resolved": "https://example.invalid/ui.tgz",
      "integrity": "{i_ui}",
      "peerDependencies": {{ "react": "^18.0.0" }}
    }}
  }}
}}"#
        ),
    )
    .unwrap();
    git_init(&project);
    let tarball = tmp.path().join("ui.tgz");
    fs::write(&tarball, &ui).unwrap();
    let source = FileArtifactSource::new(tmp.path()).with_override("ui-kit", tarball);

    init_project(&project).unwrap();
    let err = switch_project_with_source(&project, None, &source).unwrap_err();
    assert!(
        err.to_string().contains("peer") || err.to_string().contains("unsatisfied"),
        "{err}"
    );

    let doctor = doctor_project(&project).unwrap();
    assert!(doctor.has_errors());
    assert!(doctor
        .findings
        .iter()
        .any(|f| f.check == "peer-dependencies" && f.severity == DoctorSeverity::Error));
    let adoption = doctor.adoption.expect("adoption");
    assert_eq!(adoption.verdict, AdoptionVerdict::Blocked);
    assert!(adoption
        .next_actions
        .iter()
        .any(|a| a.step.contains("peer") || a.why.contains("peer")));

    std::env::remove_var("WEAVE_HOME");
}
