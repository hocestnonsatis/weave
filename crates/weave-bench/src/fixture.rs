//! Offline synthetic fixtures for Weave benchmarks.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};
use weave_fs::pack_npm_tarball;

#[derive(Debug, Clone)]
pub struct PackedPkg {
    pub name: String,
    pub version: String,
    pub tarball: PathBuf,
    pub integrity: String,
}

/// Build a tiny npm-style tarball on disk and return integrity metadata.
pub fn pack_pkg(
    out_dir: &Path,
    name: &str,
    version: &str,
    marker: &str,
) -> anyhow::Result<PackedPkg> {
    pack_pkg_with_files(out_dir, name, version, marker, 1, false, false)
}

/// Pack a package with optional native/install-script markers and extra files.
pub fn pack_pkg_with_files(
    out_dir: &Path,
    name: &str,
    version: &str,
    marker: &str,
    extra_files: usize,
    native: bool,
    install_script: bool,
) -> anyhow::Result<PackedPkg> {
    fs::create_dir_all(out_dir)?;
    let mut package: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    package.insert("name".into(), name.into());
    package.insert("version".into(), version.into());
    package.insert("main".into(), "index.js".into());
    if install_script {
        let mut scripts = serde_json::Map::new();
        scripts.insert("install".into(), "node -e \"process.exit(0)\"".into());
        package.insert("scripts".into(), serde_json::Value::Object(scripts));
        package.insert("hasInstallScript".into(), true.into());
    }
    if native {
        package.insert(
            "binary".into(),
            serde_json::json!({"module_name": name, "module_path": "./lib"}),
        );
    }
    let package_json = serde_json::to_string(&package)?;
    let index = format!("module.exports = {marker:?};\n");

    let mut entries: Vec<(String, Vec<u8>)> = vec![
        ("package.json".into(), package_json.into_bytes()),
        ("index.js".into(), index.into_bytes()),
    ];
    if native {
        entries.push(("binding.gyp".into(), b"{}".to_vec()));
        entries.push((
            "build/Release/addon.node".into(),
            b"\0fake-native\0".to_vec(),
        ));
    }
    for i in 0..extra_files.saturating_sub(1) {
        entries.push((
            format!("data/file-{i}.txt"),
            format!("{name}-{version}-payload-{i}\n").into_bytes(),
        ));
    }
    let owned: Vec<(String, Vec<u8>)> = entries;
    let refs: Vec<(&str, &[u8])> = owned
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_slice()))
        .collect();
    let bytes = pack_npm_tarball(&refs);
    let integrity = format!("sha256-{}", b64(&Sha256::digest(&bytes)));
    let filename = format!("{name}-{version}.tgz").replace('/', "-");
    let tarball = out_dir.join(filename);
    fs::write(&tarball, &bytes)?;
    Ok(PackedPkg {
        name: name.to_owned(),
        version: version.to_owned(),
        tarball,
        integrity,
    })
}

/// Scale knobs for generated dependency trees.
#[derive(Debug, Clone, Copy)]
pub struct ScaleSpec {
    pub name: &'static str,
    pub package_count: usize,
    pub extra_files_per_pkg: usize,
    /// Packages present in both env A and B (shared prefix).
    pub shared_count: usize,
    /// Additional packages unique to B.
    pub b_unique: usize,
}

pub const SCALE_TINY: ScaleSpec = ScaleSpec {
    name: "tiny",
    package_count: 2,
    extra_files_per_pkg: 1,
    shared_count: 1,
    b_unique: 1,
};

pub const SCALE_SMALL: ScaleSpec = ScaleSpec {
    name: "small",
    package_count: 25,
    extra_files_per_pkg: 3,
    shared_count: 15,
    b_unique: 10,
};

pub const SCALE_MEDIUM: ScaleSpec = ScaleSpec {
    name: "medium",
    package_count: 80,
    extra_files_per_pkg: 4,
    shared_count: 50,
    b_unique: 30,
};

pub const SCALE_LARGE: ScaleSpec = ScaleSpec {
    name: "large",
    package_count: 200,
    extra_files_per_pkg: 5,
    shared_count: 120,
    b_unique: 80,
};

/// Generated package set for a scaled suite.
pub struct ScaledPackages {
    pub shared: Vec<PackedPkg>,
    pub a_only: Vec<PackedPkg>,
    pub b_only: Vec<PackedPkg>,
    pub native: Option<PackedPkg>,
}

impl ScaledPackages {
    pub fn create(tarball_dir: &Path, spec: ScaleSpec, with_native: bool) -> anyhow::Result<Self> {
        let mut shared = Vec::new();
        for i in 0..spec.shared_count {
            shared.push(pack_pkg_with_files(
                tarball_dir,
                &format!("pkg-shared-{i:04}"),
                "1.0.0",
                &format!("shared-{i}"),
                spec.extra_files_per_pkg,
                false,
                false,
            )?);
        }
        let a_only_count = spec.package_count.saturating_sub(spec.shared_count);
        let mut a_only = Vec::new();
        for i in 0..a_only_count {
            a_only.push(pack_pkg_with_files(
                tarball_dir,
                &format!("pkg-a-{i:04}"),
                "1.0.0",
                &format!("a-{i}"),
                spec.extra_files_per_pkg,
                false,
                false,
            )?);
        }
        let mut b_only = Vec::new();
        for i in 0..spec.b_unique {
            b_only.push(pack_pkg_with_files(
                tarball_dir,
                &format!("pkg-b-{i:04}"),
                "1.0.0",
                &format!("b-{i}"),
                spec.extra_files_per_pkg,
                false,
                false,
            )?);
        }
        let native = if with_native {
            Some(pack_pkg_with_files(
                tarball_dir,
                "native-addon",
                "1.0.0",
                "native",
                2,
                true,
                true,
            )?)
        } else {
            None
        };
        Ok(Self {
            shared,
            a_only,
            b_only,
            native,
        })
    }

    pub fn source_a(&self, tarball_dir: &Path) -> weave_engine::FileArtifactSource {
        let mut src = weave_engine::FileArtifactSource::new(tarball_dir);
        for p in self.shared.iter().chain(self.a_only.iter()) {
            src = src.with_override(&p.name, p.tarball.clone());
        }
        if let Some(n) = &self.native {
            src = src.with_override(&n.name, n.tarball.clone());
        }
        src
    }

    pub fn source_b(&self, tarball_dir: &Path) -> weave_engine::FileArtifactSource {
        let mut src = weave_engine::FileArtifactSource::new(tarball_dir);
        for p in self.shared.iter().chain(self.b_only.iter()) {
            src = src.with_override(&p.name, p.tarball.clone());
        }
        if let Some(n) = &self.native {
            src = src.with_override(&n.name, n.tarball.clone());
        }
        src
    }
}

/// Write env A or B for a scaled package set.
pub fn write_scaled_project(
    root: &Path,
    env: BenchEnv,
    pkgs: &ScaledPackages,
    include_native_in_a: bool,
) -> anyhow::Result<()> {
    fs::create_dir_all(root)?;
    git_init(root)?;
    let (package_json, lockfile) = scaled_files(env, pkgs, include_native_in_a);
    fs::write(root.join("package.json"), package_json)?;
    fs::write(root.join("package-lock.json"), lockfile)?;
    fs::write(root.join("README"), format!("bench fixture {env:?}\n"))?;
    git_commit_all(root, "bench fixture")?;
    Ok(())
}

fn scaled_files(
    env: BenchEnv,
    pkgs: &ScaledPackages,
    include_native_in_a: bool,
) -> (String, String) {
    let list: Vec<&PackedPkg> = match env {
        BenchEnv::A => {
            let mut v: Vec<_> = pkgs.shared.iter().chain(pkgs.a_only.iter()).collect();
            if include_native_in_a {
                if let Some(n) = &pkgs.native {
                    v.push(n);
                }
            }
            v
        }
        BenchEnv::B => pkgs.shared.iter().chain(pkgs.b_only.iter()).collect(),
    };
    let name = match env {
        BenchEnv::A => "bench-scaled-a",
        BenchEnv::B => "bench-scaled-b",
    };
    let deps: Vec<String> = list
        .iter()
        .map(|p| format!(r#"    "{}": "{}""#, p.name, p.version))
        .collect();
    let package_json = format!(
        "{{\n  \"name\": \"{name}\",\n  \"version\": \"1.0.0\",\n  \"dependencies\": {{\n{}\n  }}\n}}\n",
        deps.join(",\n")
    );

    let mut packages_entries = Vec::new();
    packages_entries.push(format!(
        r#"    "": {{
      "name": "{name}",
      "version": "1.0.0",
      "dependencies": {{
{}
      }}
    }}"#,
        deps.join(",\n")
    ));
    for p in &list {
        let mut extra = String::new();
        if p.name == "native-addon" {
            extra.push_str(
                r#",
      "hasInstallScript": true,
      "cpu": ["x64", "arm64"],
      "os": ["linux", "darwin"]"#,
            );
        }
        let tarball_name = format!("{}-{}", p.name, p.version);
        packages_entries.push(format!(
            r#"    "node_modules/{name}": {{
      "version": "{version}",
      "resolved": "https://example.invalid/{name}/-/{tarball}.tgz",
      "integrity": "{integrity}"{extra}
    }}"#,
            name = p.name,
            version = p.version,
            tarball = tarball_name,
            integrity = p.integrity,
            extra = extra
        ));
    }
    let lockfile = format!(
        "{{\n  \"name\": \"{name}\",\n  \"version\": \"1.0.0\",\n  \"lockfileVersion\": 3,\n  \"packages\": {{\n{}\n  }}\n}}\n",
        packages_entries.join(",\n")
    );
    (package_json, lockfile)
}

/// Initialize a git repo with package.json + lockfile for env A or B (legacy tiny).
pub fn write_project(root: &Path, env: BenchEnv, pkgs: &BenchPackages) -> anyhow::Result<()> {
    fs::create_dir_all(root)?;
    git_init(root)?;

    let (package_json, lockfile) = match env {
        BenchEnv::A => env_a_files(pkgs),
        BenchEnv::B => env_b_files(pkgs),
    };
    fs::write(root.join("package.json"), package_json)?;
    fs::write(root.join("package-lock.json"), lockfile)?;
    fs::write(root.join("README"), format!("bench fixture {env:?}\n"))?;
    git_commit_all(root, "bench fixture")?;
    Ok(())
}

/// Write a simple npm workspaces monorepo lockfile (v3 style).
pub fn write_monorepo_project(root: &Path, pkgs: &[PackedPkg]) -> anyhow::Result<()> {
    fs::create_dir_all(root.join("packages/app"))?;
    fs::create_dir_all(root.join("packages/lib"))?;
    git_init(root)?;

    let root_pkg = r#"{
  "name": "bench-monorepo",
  "version": "1.0.0",
  "private": true,
  "workspaces": ["packages/*"]
}
"#;
    fs::write(root.join("package.json"), root_pkg)?;
    fs::write(
        root.join("packages/app/package.json"),
        r#"{"name":"@bench/app","version":"1.0.0","dependencies":{"@bench/lib":"1.0.0"}}
"#,
    )?;
    fs::write(
        root.join("packages/lib/package.json"),
        r#"{"name":"@bench/lib","version":"1.0.0"}
"#,
    )?;

    // Flat deps from pkgs attached to root for materialization volume.
    let deps: Vec<String> = pkgs
        .iter()
        .map(|p| format!(r#"    "{}": "{}""#, p.name, p.version))
        .collect();
    let mut packages_entries = vec![format!(
        r#"    "": {{
      "name": "bench-monorepo",
      "version": "1.0.0",
      "workspaces": ["packages/*"],
      "dependencies": {{
{}
      }}
    }},
    "packages/app": {{
      "name": "@bench/app",
      "version": "1.0.0",
      "dependencies": {{ "@bench/lib": "1.0.0" }}
    }},
    "packages/lib": {{
      "name": "@bench/lib",
      "version": "1.0.0"
    }},
    "node_modules/@bench/app": {{ "resolved": "packages/app", "link": true }},
    "node_modules/@bench/lib": {{ "resolved": "packages/lib", "link": true }}"#,
        deps.join(",\n")
    )];
    for p in pkgs {
        let tarball_name = format!("{}-{}", p.name, p.version);
        packages_entries.push(format!(
            r#"    "node_modules/{name}": {{
      "version": "{version}",
      "resolved": "https://example.invalid/{name}/-/{tarball}.tgz",
      "integrity": "{integrity}"
    }}"#,
            name = p.name,
            version = p.version,
            tarball = tarball_name,
            integrity = p.integrity
        ));
    }
    let lockfile = format!(
        "{{\n  \"name\": \"bench-monorepo\",\n  \"version\": \"1.0.0\",\n  \"lockfileVersion\": 3,\n  \"packages\": {{\n{}\n  }}\n}}\n",
        packages_entries.join(",\n")
    );
    fs::write(root.join("package-lock.json"), lockfile)?;
    fs::write(root.join("README"), "monorepo bench\n")?;
    git_commit_all(root, "monorepo fixture")?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub enum BenchEnv {
    A,
    B,
}

pub struct BenchPackages {
    pub demo: PackedPkg,
    pub shared_v1: PackedPkg,
    pub shared_v2: PackedPkg,
    pub extra: PackedPkg,
}

impl BenchPackages {
    pub fn create(tarball_dir: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            demo: pack_pkg(tarball_dir, "demo-pkg", "1.0.0", "demo")?,
            shared_v1: pack_pkg(tarball_dir, "shared", "1.0.0", "shared-1")?,
            shared_v2: pack_pkg(tarball_dir, "shared", "2.0.0", "shared-2")?,
            extra: pack_pkg(tarball_dir, "extra", "1.0.0", "extra")?,
        })
    }
}

fn env_a_files(pkgs: &BenchPackages) -> (String, String) {
    let package_json = r#"{
  "name": "bench-small-a",
  "version": "1.0.0",
  "dependencies": {
    "demo-pkg": "1.0.0",
    "shared": "1.0.0"
  }
}
"#
    .to_owned();
    let lockfile = format!(
        r#"{{
  "name": "bench-small-a",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "packages": {{
    "": {{
      "name": "bench-small-a",
      "version": "1.0.0",
      "dependencies": {{
        "demo-pkg": "1.0.0",
        "shared": "1.0.0"
      }}
    }},
    "node_modules/demo-pkg": {{
      "version": "1.0.0",
      "resolved": "https://example.invalid/demo-pkg/-/demo-pkg-1.0.0.tgz",
      "integrity": "{demo_int}"
    }},
    "node_modules/shared": {{
      "version": "1.0.0",
      "resolved": "https://example.invalid/shared/-/shared-1.0.0.tgz",
      "integrity": "{shared1_int}"
    }}
  }}
}}
"#,
        demo_int = pkgs.demo.integrity,
        shared1_int = pkgs.shared_v1.integrity,
    );
    (package_json, lockfile)
}

fn env_b_files(pkgs: &BenchPackages) -> (String, String) {
    let package_json = r#"{
  "name": "bench-small-b",
  "version": "1.0.0",
  "dependencies": {
    "demo-pkg": "1.0.0",
    "shared": "2.0.0",
    "extra": "1.0.0"
  }
}
"#
    .to_owned();
    let lockfile = format!(
        r#"{{
  "name": "bench-small-b",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "packages": {{
    "": {{
      "name": "bench-small-b",
      "version": "1.0.0",
      "dependencies": {{
        "demo-pkg": "1.0.0",
        "shared": "2.0.0",
        "extra": "1.0.0"
      }}
    }},
    "node_modules/demo-pkg": {{
      "version": "1.0.0",
      "resolved": "https://example.invalid/demo-pkg/-/demo-pkg-1.0.0.tgz",
      "integrity": "{demo_int}"
    }},
    "node_modules/shared": {{
      "version": "2.0.0",
      "resolved": "https://example.invalid/shared/-/shared-2.0.0.tgz",
      "integrity": "{shared2_int}"
    }},
    "node_modules/extra": {{
      "version": "1.0.0",
      "resolved": "https://example.invalid/extra/-/extra-1.0.0.tgz",
      "integrity": "{extra_int}"
    }}
  }}
}}
"#,
        demo_int = pkgs.demo.integrity,
        shared2_int = pkgs.shared_v2.integrity,
        extra_int = pkgs.extra.integrity,
    );
    (package_json, lockfile)
}

fn git_init(root: &Path) -> anyhow::Result<()> {
    let status = Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(root)
        .status()?;
    anyhow::ensure!(status.success(), "git init failed");
    let _ = Command::new("git")
        .args(["config", "user.email", "weave-bench@example.com"])
        .current_dir(root)
        .status()?;
    let _ = Command::new("git")
        .args(["config", "user.name", "Weave Bench"])
        .current_dir(root)
        .status()?;
    Ok(())
}

fn git_commit_all(root: &Path, message: &str) -> anyhow::Result<()> {
    let status = Command::new("git")
        .args(["add", "."])
        .current_dir(root)
        .status()?;
    anyhow::ensure!(status.success(), "git add failed");
    let status = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(root)
        .status()?;
    anyhow::ensure!(status.success(), "git commit failed");
    Ok(())
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
