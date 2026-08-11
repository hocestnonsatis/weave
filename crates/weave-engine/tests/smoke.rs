//! Phase 4 application smoke tests (offline, no lifecycle script execution).
//!
//! Distinguishes materialization correctness from lifecycle-generated state:
//! these tests never run install scripts.

use std::fs;
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

use sha2::{Digest, Sha256};
use weave_engine::{init_project, switch_project_with_source, FileArtifactSource};
use weave_fs::pack_npm_tarball;

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
    let status = Command::new("git")
        .args(["init"])
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success());
    let _ = Command::new("git")
        .args(["config", "user.email", "weave@test"])
        .current_dir(dir)
        .status();
    let _ = Command::new("git")
        .args(["config", "user.name", "weave"])
        .current_dir(dir)
        .status();
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
fn smoke_bins_exports_nested_and_node_start() {
    let _guard = lock_home();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
    let project = tmp.path().join("app");
    fs::create_dir_all(&project).unwrap();

    let cli = pack_npm_tarball(&[
        (
            "package.json",
            br#"{"name":"demo-cli","version":"1.0.0","bin":{"demo-cli":"cli.js"}}"#,
        ),
        ("cli.js", b"#!/usr/bin/env node\nconsole.log('bin-ok');\n"),
        ("index.js", b"module.exports = 'cli';\n"),
    ]);
    let exports_pkg = pack_npm_tarball(&[
        (
            "package.json",
            br##"{"name":"exports-pkg","version":"1.0.0","exports":{".":"./main.js","./sub":"./sub.js"},"imports":{"#internal":"./main.js"}}"##,
        ),
        ("main.js", b"module.exports = { root: true };\n"),
        ("sub.js", b"module.exports = { sub: true };\n"),
    ]);
    let nested = pack_npm_tarball(&[
        (
            "package.json",
            br#"{"name":"nested-lib","version":"1.0.0"}"#,
        ),
        ("index.js", b"module.exports = 'nested';\n"),
    ]);
    let parent = pack_npm_tarball(&[
        (
            "package.json",
            br#"{"name":"parent","version":"1.0.0","dependencies":{"nested-lib":"1.0.0"}}"#,
        ),
        ("index.js", b"module.exports = require('nested-lib');\n"),
    ]);

    let i_cli = sri(&cli);
    let i_exp = sri(&exports_pkg);
    let i_nested = sri(&nested);
    let i_parent = sri(&parent);

    fs::write(
        project.join("package.json"),
        r#"{"name":"smoke-app","version":"1.0.0","type":"commonjs"}"#,
    )
    .unwrap();
    fs::write(
        project.join("package-lock.json"),
        format!(
            r#"{{
  "name": "smoke-app",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "packages": {{
    "": {{
      "name": "smoke-app",
      "version": "1.0.0",
      "dependencies": {{
        "demo-cli": "1.0.0",
        "exports-pkg": "1.0.0",
        "parent": "1.0.0"
      }}
    }},
    "node_modules/demo-cli": {{
      "version": "1.0.0",
      "resolved": "https://example.invalid/demo-cli.tgz",
      "integrity": "{i_cli}",
      "bin": {{ "demo-cli": "cli.js" }}
    }},
    "node_modules/exports-pkg": {{
      "version": "1.0.0",
      "resolved": "https://example.invalid/exports-pkg.tgz",
      "integrity": "{i_exp}"
    }},
    "node_modules/parent": {{
      "version": "1.0.0",
      "resolved": "https://example.invalid/parent.tgz",
      "integrity": "{i_parent}",
      "dependencies": {{ "nested-lib": "1.0.0" }}
    }},
    "node_modules/parent/node_modules/nested-lib": {{
      "version": "1.0.0",
      "resolved": "https://example.invalid/nested-lib.tgz",
      "integrity": "{i_nested}"
    }}
  }}
}}"#
        ),
    )
    .unwrap();
    fs::write(
        project.join("app.js"),
        r#"
const exp = require('exports-pkg');
const sub = require('exports-pkg/sub');
const parent = require('parent');
if (!exp.root) throw new Error('exports root failed');
if (!sub.sub) throw new Error('exports subpath failed');
if (parent !== 'nested') throw new Error('nested resolve failed');
console.log('smoke-ok');
"#,
    )
    .unwrap();
    git_init(&project);

    let tarballs = tmp.path().join("tarballs");
    fs::create_dir_all(&tarballs).unwrap();
    fs::write(tarballs.join("demo-cli.tgz"), &cli).unwrap();
    fs::write(tarballs.join("exports-pkg.tgz"), &exports_pkg).unwrap();
    fs::write(tarballs.join("parent.tgz"), &parent).unwrap();
    fs::write(tarballs.join("nested-lib.tgz"), &nested).unwrap();

    let mut source = FileArtifactSource::new(&tarballs);
    source = source.with_override("demo-cli", tarballs.join("demo-cli.tgz"));
    source = source.with_override("exports-pkg", tarballs.join("exports-pkg.tgz"));
    source = source.with_override("parent", tarballs.join("parent.tgz"));
    source = source.with_override("nested-lib", tarballs.join("nested-lib.tgz"));

    init_project(&project).unwrap();
    switch_project_with_source(&project, None, &source).unwrap();

    assert!(project.join("node_modules/.bin/demo-cli").exists());
    let bin_out = Command::new(project.join("node_modules/.bin/demo-cli"))
        .current_dir(&project)
        .output()
        .unwrap();
    assert!(
        bin_out.status.success(),
        "bin failed: {}",
        String::from_utf8_lossy(&bin_out.stderr)
    );
    assert!(String::from_utf8_lossy(&bin_out.stdout).contains("bin-ok"));

    let node_out = Command::new("node")
        .arg("app.js")
        .current_dir(&project)
        .output()
        .unwrap();
    assert!(
        node_out.status.success(),
        "node failed: {}",
        String::from_utf8_lossy(&node_out.stderr)
    );
    assert!(String::from_utf8_lossy(&node_out.stdout).contains("smoke-ok"));

    std::env::remove_var("WEAVE_HOME");
}

#[test]
fn smoke_file_dep_snapshot_and_workspace_links() {
    let _guard = lock_home();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
    let project = tmp.path().join("mono");
    fs::create_dir_all(project.join("packages/a")).unwrap();
    fs::create_dir_all(project.join("packages/b")).unwrap();
    fs::create_dir_all(project.join("vendor/local-lib")).unwrap();

    fs::write(
        project.join("vendor/local-lib/package.json"),
        r#"{"name":"local-lib","version":"1.0.0"}"#,
    )
    .unwrap();
    fs::write(
        project.join("vendor/local-lib/index.js"),
        b"module.exports = 'local-snap';\n",
    )
    .unwrap();

    fs::write(
        project.join("packages/b/package.json"),
        r#"{"name":"@acme/b","version":"1.0.0"}"#,
    )
    .unwrap();
    fs::write(
        project.join("packages/b/index.js"),
        b"module.exports = 'from-b';\n",
    )
    .unwrap();
    fs::write(
        project.join("packages/a/package.json"),
        r#"{"name":"@acme/a","version":"1.0.0","dependencies":{"@acme/b":"*"}}"#,
    )
    .unwrap();
    fs::write(
        project.join("packages/a/index.js"),
        b"module.exports = require('@acme/b');\n",
    )
    .unwrap();

    fs::write(
        project.join("package.json"),
        r#"{"name":"mono","version":"1.0.0","workspaces":["packages/*"],"dependencies":{"@acme/a":"*","local-lib":"file:vendor/local-lib"}}"#,
    )
    .unwrap();
    fs::write(
        project.join("package-lock.json"),
        r#"{
  "name": "mono",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "packages": {
    "": {
      "name": "mono",
      "version": "1.0.0",
      "workspaces": ["packages/a", "packages/b"],
      "dependencies": {
        "@acme/a": "*",
        "local-lib": "file:vendor/local-lib"
      }
    },
    "node_modules/@acme/a": { "resolved": "packages/a", "link": true },
    "node_modules/@acme/b": { "resolved": "packages/b", "link": true },
    "node_modules/local-lib": {
      "version": "1.0.0",
      "resolved": "file:vendor/local-lib"
    },
    "packages/a": {
      "name": "@acme/a",
      "version": "1.0.0",
      "dependencies": { "@acme/b": "*" }
    },
    "packages/b": { "name": "@acme/b", "version": "1.0.0" }
  }
}"#,
    )
    .unwrap();
    fs::write(
        project.join("app.js"),
        r#"
const a = require('@acme/a');
const local = require('local-lib');
if (a !== 'from-b') throw new Error('workspace resolve failed: ' + a);
if (local !== 'local-snap') throw new Error('file dep failed: ' + local);
console.log('workspace-file-ok');
"#,
    )
    .unwrap();
    git_init(&project);

    // Mutate vendor after we will snapshot — proves immutability of acquire.
    init_project(&project).unwrap();
    let source = FileArtifactSource::new(&project);
    switch_project_with_source(&project, None, &source).unwrap();

    assert!(project
        .join("node_modules/@acme/a")
        .symlink_metadata()
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(project.join("node_modules/local-lib/index.js").is_file());

    // Change vendor source; activated tree must keep snapshot content.
    fs::write(
        project.join("vendor/local-lib/index.js"),
        b"module.exports = 'mutated';\n",
    )
    .unwrap();
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
    assert!(String::from_utf8_lossy(&out.stdout).contains("workspace-file-ok"));
    assert_eq!(
        fs::read_to_string(project.join("node_modules/local-lib/index.js")).unwrap(),
        "module.exports = 'local-snap';\n"
    );

    std::env::remove_var("WEAVE_HOME");
}

#[test]
fn smoke_peers_and_optional_platform_filter() {
    let _guard = lock_home();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
    let project = tmp.path().join("peers");
    fs::create_dir_all(&project).unwrap();

    let react = pack_npm_tarball(&[
        ("package.json", br#"{"name":"react","version":"18.2.0"}"#),
        ("index.js", b"module.exports = { react: true };\n"),
    ]);
    let ui = pack_npm_tarball(&[
        (
            "package.json",
            br#"{"name":"ui-kit","version":"1.0.0","peerDependencies":{"react":"^18.0.0"}}"#,
        ),
        ("index.js", b"module.exports = require('react');\n"),
    ]);
    let core = pack_npm_tarball(&[
        ("package.json", br#"{"name":"core","version":"1.0.0"}"#),
        ("index.js", b"module.exports = 'core';\n"),
    ]);
    let fsevents = pack_npm_tarball(&[
        (
            "package.json",
            br#"{"name":"fsevents","version":"2.3.3","os":["darwin"]}"#,
        ),
        ("index.js", b"module.exports = 'darwin-only';\n"),
    ]);

    let i_react = sri(&react);
    let i_ui = sri(&ui);
    let i_core = sri(&core);
    let i_fs = sri(&fsevents);

    fs::write(
        project.join("package.json"),
        r#"{"name":"peers-app","version":"1.0.0"}"#,
    )
    .unwrap();
    fs::write(
        project.join("package-lock.json"),
        format!(
            r#"{{
  "name": "peers-app",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "packages": {{
    "": {{
      "name": "peers-app",
      "version": "1.0.0",
      "dependencies": {{
        "react": "18.2.0",
        "ui-kit": "1.0.0",
        "core": "1.0.0"
      }},
      "optionalDependencies": {{
        "fsevents": "2.3.3"
      }}
    }},
    "node_modules/react": {{
      "version": "18.2.0",
      "resolved": "https://example.invalid/react.tgz",
      "integrity": "{i_react}"
    }},
    "node_modules/ui-kit": {{
      "version": "1.0.0",
      "resolved": "https://example.invalid/ui-kit.tgz",
      "integrity": "{i_ui}",
      "peerDependencies": {{ "react": "^18.0.0" }}
    }},
    "node_modules/core": {{
      "version": "1.0.0",
      "resolved": "https://example.invalid/core.tgz",
      "integrity": "{i_core}"
    }},
    "node_modules/fsevents": {{
      "version": "2.3.3",
      "resolved": "https://example.invalid/fsevents.tgz",
      "integrity": "{i_fs}",
      "optional": true,
      "os": ["darwin"]
    }}
  }}
}}"#
        ),
    )
    .unwrap();
    fs::write(
        project.join("app.js"),
        r#"
const ui = require('ui-kit');
const core = require('core');
if (!ui.react) throw new Error('peer resolve failed');
if (core !== 'core') throw new Error('core failed');
try {
  require('fsevents');
  if (process.platform === 'linux') throw new Error('fsevents should be skipped on linux');
} catch (e) {
  if (e.code !== 'MODULE_NOT_FOUND' && process.platform === 'linux') throw e;
}
console.log('peers-optional-ok');
"#,
    )
    .unwrap();
    git_init(&project);

    let tarballs = tmp.path().join("tarballs");
    fs::create_dir_all(&tarballs).unwrap();
    for (name, bytes) in [
        ("react", &react),
        ("ui-kit", &ui),
        ("core", &core),
        ("fsevents", &fsevents),
    ] {
        fs::write(tarballs.join(format!("{name}.tgz")), bytes).unwrap();
    }
    let mut source = FileArtifactSource::new(&tarballs);
    source = source.with_override("react", tarballs.join("react.tgz"));
    source = source.with_override("ui-kit", tarballs.join("ui-kit.tgz"));
    source = source.with_override("core", tarballs.join("core.tgz"));
    source = source.with_override("fsevents", tarballs.join("fsevents.tgz"));

    init_project(&project).unwrap();
    switch_project_with_source(&project, None, &source).unwrap();

    if cfg!(target_os = "linux") {
        assert!(
            !project.join("node_modules/fsevents").exists(),
            "darwin-only optional package must be skipped on linux"
        );
    }
    assert!(project.join("node_modules/ui-kit").is_dir());
    assert!(project.join("node_modules/react").is_dir());

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
    assert!(String::from_utf8_lossy(&out.stdout).contains("peers-optional-ok"));

    std::env::remove_var("WEAVE_HOME");
}

#[test]
fn smoke_rejects_missing_required_peer() {
    let _guard = lock_home();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
    let project = tmp.path().join("bad-peer");
    fs::create_dir_all(&project).unwrap();

    let ui = pack_npm_tarball(&[
        (
            "package.json",
            br#"{"name":"ui-kit","version":"1.0.0","peerDependencies":{"react":"^18.0.0"}}"#,
        ),
        ("index.js", b"module.exports = 1;\n"),
    ]);
    let i_ui = sri(&ui);
    fs::write(
        project.join("package.json"),
        r#"{"name":"bad","version":"1.0.0"}"#,
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
    let msg = err.to_string();
    assert!(
        msg.contains("peer") || msg.contains("unsatisfied"),
        "unexpected error: {msg}"
    );
    std::env::remove_var("WEAVE_HOME");
}
