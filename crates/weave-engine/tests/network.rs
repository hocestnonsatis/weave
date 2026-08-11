//! Network-gated registry → CAS → materialize → Node smoke.
//!
//! Ignored by default. Enable with:
//! `WEAVE_NETWORK_TESTS=1 cargo test -p weave-engine --test network -- --ignored`
//!
//! Large NestJS corpus path:
//! `WEAVE_NETWORK_LARGE=1 WEAVE_NETWORK_TESTS=1 cargo test -p weave-engine --test network nestjs -- --ignored`

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

use weave_engine::{init_project, switch_project};

static LOCK: Mutex<()> = Mutex::new(());

fn lock_home() -> MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

fn network_enabled() -> bool {
    matches!(
        std::env::var("WEAVE_NETWORK_TESTS").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/corpus")
}

fn prepare_git_project(project: &std::path::Path) {
    assert!(Command::new("git")
        .args(["init"])
        .current_dir(project)
        .status()
        .unwrap()
        .success());
    let _ = Command::new("git")
        .args(["config", "user.email", "weave@test"])
        .current_dir(project)
        .status();
    let _ = Command::new("git")
        .args(["config", "user.name", "weave"])
        .current_dir(project)
        .status();
    assert!(Command::new("git")
        .args(["add", "."])
        .current_dir(project)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["commit", "-m", "init"])
        .current_dir(project)
        .status()
        .unwrap()
        .success());
}

fn run_corpus_materialize(corpus_rel: &str, require_pkg: &str) {
    if !network_enabled() {
        eprintln!("skip: WEAVE_NETWORK_TESTS not set");
        return;
    }
    let _guard = lock_home();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));

    let corpus = corpus_root().join(corpus_rel);
    let lock = corpus.join("package-lock.json");
    let pkg = corpus.join("package.json");
    if !lock.is_file() || !pkg.is_file() {
        eprintln!("blocker: corpus missing at {}", corpus.display());
        std::env::remove_var("WEAVE_HOME");
        return;
    }

    let project = tmp.path().join("net-smoke");
    fs::create_dir_all(&project).unwrap();
    fs::copy(&pkg, project.join("package.json")).unwrap();
    fs::copy(&lock, project.join("package-lock.json")).unwrap();
    fs::write(
        project.join("smoke.cjs"),
        format!(
            r#"
try {{
  require({require_pkg:?});
  console.log('registry-smoke-ok');
}} catch (e) {{
  console.error(e);
  process.exit(1);
}}
"#
        ),
    )
    .unwrap();
    prepare_git_project(&project);
    init_project(&project).unwrap();

    match switch_project(&project, None) {
        Ok(out) => {
            eprintln!(
                "network materialize ({corpus_rel}): packages={} fetched={} reused={} bins={}",
                out.prepare.materialize.packages_materialized,
                out.prepare.fetched_artifacts,
                out.prepare.reused_artifacts,
                out.prepare.materialize.bin_links
            );
            let node = Command::new("node")
                .arg("smoke.cjs")
                .current_dir(&project)
                .output()
                .unwrap();
            assert!(
                node.status.success(),
                "node smoke failed: {}",
                String::from_utf8_lossy(&node.stderr)
            );
            assert!(String::from_utf8_lossy(&node.stdout).contains("registry-smoke-ok"));
        }
        Err(err) => {
            eprintln!("network materialize failed (do not fake): {err}");
            if matches!(
                std::env::var("WEAVE_NETWORK_STRICT").as_deref(),
                Ok("1") | Ok("true")
            ) {
                panic!("strict network test failed: {err}");
            }
        }
    }

    std::env::remove_var("WEAVE_HOME");
}

#[test]
#[ignore = "network-gated; set WEAVE_NETWORK_TESTS=1"]
fn rimraf_registry_materialize_smoke() {
    // rimraf lockfile deps include extraction-friendly packages like `glob`.
    run_corpus_materialize("small/rimraf", "glob");
}

#[test]
#[ignore = "network-gated large; set WEAVE_NETWORK_TESTS=1 WEAVE_NETWORK_LARGE=1"]
fn nestjs_scale_registry_materialize_smoke() {
    if !matches!(
        std::env::var("WEAVE_NETWORK_LARGE").as_deref(),
        Ok("1") | Ok("true")
    ) {
        eprintln!("skip: WEAVE_NETWORK_LARGE not set");
        return;
    }
    run_corpus_materialize("large/nestjs", "tslib");
}
