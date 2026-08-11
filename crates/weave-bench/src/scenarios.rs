//! Offline / optional package-manager benchmark suites.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use weave_engine::{init_project, switch_project_with_source, FileArtifactSource, ProjectConfig};

use crate::fixture::{
    write_monorepo_project, write_project, write_scaled_project, BenchEnv, BenchPackages,
    ScaleSpec, ScaledPackages, SCALE_LARGE, SCALE_MEDIUM, SCALE_SMALL, SCALE_TINY,
};
use crate::measure::{time_it, tree_stats, trees_stats, HostInfo, ScenarioResult};
use crate::report::BenchSuiteResult;

/// Comparison backends for optional package-manager rows.
#[derive(Debug, Clone, Copy, Default)]
pub struct CompareFlags {
    pub npm: bool,
    pub pnpm: bool,
}

/// Which suite(s) to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuiteKind {
    Tiny,
    Small,
    Medium,
    Large,
    Monorepo,
    Divergence,
    Native,
    AllOffline,
}

impl SuiteKind {
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        Ok(match s {
            "tiny" => Self::Tiny,
            "small" => Self::Small,
            "medium" => Self::Medium,
            "large" => Self::Large,
            "monorepo" => Self::Monorepo,
            "divergence" => Self::Divergence,
            "native" => Self::Native,
            "all" | "all-offline" => Self::AllOffline,
            other => anyhow::bail!(
                "unknown suite '{other}' (tiny|small|medium|large|monorepo|divergence|native|all)"
            ),
        })
    }
}

/// Run one or more suites; concatenate rows when `AllOffline`.
pub fn run_suite(
    kind: SuiteKind,
    compare: CompareFlags,
    keep_work: bool,
) -> anyhow::Result<BenchSuiteResult> {
    match kind {
        SuiteKind::Tiny => run_tiny(compare, keep_work),
        SuiteKind::Small => run_scaled(SCALE_SMALL, compare, keep_work, false),
        SuiteKind::Medium => run_scaled(SCALE_MEDIUM, compare, keep_work, false),
        SuiteKind::Large => run_scaled(SCALE_LARGE, compare, keep_work, false),
        SuiteKind::Monorepo => run_monorepo(compare, keep_work),
        SuiteKind::Divergence => run_scaled(SCALE_SMALL, compare, keep_work, false),
        SuiteKind::Native => run_scaled(
            ScaleSpec {
                name: "native",
                package_count: 8,
                extra_files_per_pkg: 2,
                shared_count: 5,
                b_unique: 3,
            },
            compare,
            keep_work,
            true,
        ),
        SuiteKind::AllOffline => {
            let mut combined = BenchSuiteResult {
                suite: "all-offline".into(),
                host: HostInfo::capture(),
                work_dir: String::new(),
                rows: Vec::new(),
                summary: None,
            };
            for k in [
                SuiteKind::Tiny,
                SuiteKind::Small,
                SuiteKind::Medium,
                SuiteKind::Monorepo,
                SuiteKind::Native,
            ] {
                // Skip large in default all — too slow for CI smoke; run explicitly.
                let part = run_suite(k, CompareFlags::default(), false)?;
                for mut row in part.rows {
                    row.name = format!("{}::{}", part.suite, row.name);
                    combined.rows.push(row);
                }
                if combined.work_dir.is_empty() {
                    combined.work_dir = part.work_dir;
                }
            }
            combined.summary = Some(summarize_switch_metrics(&combined.rows));
            Ok(combined)
        }
    }
}

fn run_tiny(compare: CompareFlags, keep_work: bool) -> anyhow::Result<BenchSuiteResult> {
    let (root, _td) = make_work_root(keep_work)?;
    let weave_home = root.join("weave-home");
    let tarball_dir = root.join("tarballs");
    fs::create_dir_all(&weave_home)?;
    std::env::set_var("WEAVE_HOME", &weave_home);

    let pkgs = BenchPackages::create(&tarball_dir)?;
    let project_a = root.join("project-a");
    let project_b = root.join("project-b");
    write_project(&project_a, BenchEnv::A, &pkgs)?;
    write_project(&project_b, BenchEnv::B, &pkgs)?;

    let source_a = FileArtifactSource::new(&tarball_dir)
        .with_override("demo-pkg", pkgs.demo.tarball.clone())
        .with_override("shared", pkgs.shared_v1.tarball.clone());
    let source_b = FileArtifactSource::new(&tarball_dir)
        .with_override("demo-pkg", pkgs.demo.tarball.clone())
        .with_override("shared", pkgs.shared_v2.tarball.clone())
        .with_override("extra", pkgs.extra.tarball.clone());

    let mut rows = measure_cold_warm_switch(&project_a, &project_b, &source_a, &source_b)?;

    if compare.npm {
        rows.push(run_pm_ci(
            &root,
            "npm-ci",
            &["ci", "--ignore-scripts"],
            &project_a,
            &[
                (
                    "https://example.invalid/demo-pkg/-/demo-pkg-1.0.0.tgz",
                    pkgs.demo.tarball.as_path(),
                ),
                (
                    "https://example.invalid/shared/-/shared-1.0.0.tgz",
                    pkgs.shared_v1.tarball.as_path(),
                ),
            ],
        )?);
    }
    if compare.pnpm {
        rows.push(run_pm_ci(
            &root,
            "pnpm-install-frozen",
            &["install", "--frozen-lockfile", "--ignore-scripts"],
            &project_a,
            &[
                (
                    "https://example.invalid/demo-pkg/-/demo-pkg-1.0.0.tgz",
                    pkgs.demo.tarball.as_path(),
                ),
                (
                    "https://example.invalid/shared/-/shared-1.0.0.tgz",
                    pkgs.shared_v1.tarball.as_path(),
                ),
            ],
        )?);
    }

    finish_suite(SCALE_TINY.name, root, rows, keep_work)
}

fn run_scaled(
    spec: ScaleSpec,
    compare: CompareFlags,
    keep_work: bool,
    with_native: bool,
) -> anyhow::Result<BenchSuiteResult> {
    let (root, _td) = make_work_root(keep_work)?;
    let weave_home = root.join("weave-home");
    let tarball_dir = root.join("tarballs");
    fs::create_dir_all(&weave_home)?;
    std::env::set_var("WEAVE_HOME", &weave_home);

    let pkgs = ScaledPackages::create(&tarball_dir, spec, with_native)?;
    let project_a = root.join("project-a");
    let project_b = root.join("project-b");
    write_scaled_project(&project_a, BenchEnv::A, &pkgs, with_native)?;
    write_scaled_project(&project_b, BenchEnv::B, &pkgs, false)?;

    let source_a = pkgs.source_a(&tarball_dir);
    let source_b = pkgs.source_b(&tarball_dir);

    let mut rows = measure_cold_warm_switch(&project_a, &project_b, &source_a, &source_b)?;

    if with_native {
        if let Some(note) = rows.iter_mut().find(|r| r.name == "weave-cold") {
            note.note = Some(format!(
                "{}; native-addon prefer_copy expected",
                note.note.clone().unwrap_or_default()
            ));
        }
    }

    if compare.npm && spec.package_count <= 40 {
        let mut rewrites = Vec::new();
        for p in pkgs.shared.iter().chain(pkgs.a_only.iter()) {
            let tarball_name = format!("{}-{}", p.name, p.version);
            let url = format!("https://example.invalid/{}/-/{}.tgz", p.name, tarball_name);
            rewrites.push((url, p.tarball.clone()));
        }
        let refs: Vec<(&str, &Path)> = rewrites
            .iter()
            .map(|(u, p)| (u.as_str(), p.as_path()))
            .collect();
        rows.push(run_pm_ci(
            &root,
            "npm-ci",
            &["ci", "--ignore-scripts"],
            &project_a,
            &refs,
        )?);
    }

    finish_suite(spec.name, root, rows, keep_work)
}

fn run_monorepo(compare: CompareFlags, keep_work: bool) -> anyhow::Result<BenchSuiteResult> {
    let (root, _td) = make_work_root(keep_work)?;
    let weave_home = root.join("weave-home");
    let tarball_dir = root.join("tarballs");
    fs::create_dir_all(&weave_home)?;
    std::env::set_var("WEAVE_HOME", &weave_home);

    let mut pkgs = Vec::new();
    for i in 0..12 {
        pkgs.push(crate::fixture::pack_pkg_with_files(
            &tarball_dir,
            &format!("mono-dep-{i:02}"),
            "1.0.0",
            &format!("mono-{i}"),
            2,
            false,
            false,
        )?);
    }
    let project = root.join("monorepo");
    write_monorepo_project(&project, &pkgs)?;
    let mut source = FileArtifactSource::new(&tarball_dir);
    for p in &pkgs {
        source = source.with_override(&p.name, p.tarball.clone());
    }

    let mut rows = Vec::new();
    init_project(&project).map_err(anyhow::Error::msg)?;
    let (outcome, dur) = time_it(|| switch_project_with_source(&project, None, &source));
    let outcome = outcome.map_err(anyhow::Error::msg)?;
    let (disk, files, inodes) = store_and_nm_stats(&project)?;
    rows.push(ScenarioResult {
        name: "weave-cold".into(),
        wall_ms: dur.as_millis(),
        disk_bytes: Some(disk),
        file_count: Some(files),
        approx_inodes: Some(inodes),
        note: Some(format!(
            "monorepo packages={} fetched={}",
            outcome.prepare.materialize.packages_materialized, outcome.prepare.fetched_artifacts
        )),
    });

    let (outcome, dur) = time_it(|| switch_project_with_source(&project, None, &source));
    let outcome = outcome.map_err(anyhow::Error::msg)?;
    let (disk, files, inodes) = tree_stats(&project.join("node_modules"));
    rows.push(ScenarioResult {
        name: "weave-warm".into(),
        wall_ms: dur.as_millis(),
        disk_bytes: Some(disk),
        file_count: Some(files),
        approx_inodes: Some(inodes),
        note: Some(format!(
            "cache_hits={}",
            outcome.prepare.materialize.cache_hits
        )),
    });

    let _ = compare;
    finish_suite("monorepo", root, rows, keep_work)
}

fn measure_cold_warm_switch(
    project_a: &Path,
    project_b: &Path,
    source_a: &FileArtifactSource,
    source_b: &FileArtifactSource,
) -> anyhow::Result<Vec<ScenarioResult>> {
    let mut rows = Vec::new();

    init_project(project_a).map_err(anyhow::Error::msg)?;
    let (outcome, dur) = time_it(|| switch_project_with_source(project_a, None, source_a));
    let outcome = outcome.map_err(anyhow::Error::msg)?;
    let (disk, files, inodes) = store_and_nm_stats(project_a)?;
    rows.push(ScenarioResult {
        name: "weave-cold".into(),
        wall_ms: dur.as_millis(),
        disk_bytes: Some(disk),
        file_count: Some(files),
        approx_inodes: Some(inodes),
        note: Some(format!(
            "fetched={} reused={} hardlinks={} copies={}",
            outcome.prepare.fetched_artifacts,
            outcome.prepare.reused_artifacts,
            outcome.prepare.materialize.hardlinked_files,
            outcome.prepare.materialize.copied_files
        )),
    });

    let (outcome, dur) = time_it(|| switch_project_with_source(project_a, None, source_a));
    let outcome = outcome.map_err(anyhow::Error::msg)?;
    let (disk, files, inodes) = tree_stats(&project_a.join("node_modules"));
    rows.push(ScenarioResult {
        name: "weave-warm".into(),
        wall_ms: dur.as_millis(),
        disk_bytes: Some(disk),
        file_count: Some(files),
        approx_inodes: Some(inodes),
        note: Some(format!(
            "fetched={} reused={} cache_hits={}",
            outcome.prepare.fetched_artifacts,
            outcome.prepare.reused_artifacts,
            outcome.prepare.materialize.cache_hits
        )),
    });

    init_project(project_b).map_err(anyhow::Error::msg)?;
    switch_project_with_source(project_b, None, source_b).map_err(anyhow::Error::msg)?;

    let pj_a = fs::read_to_string(project_a.join("package.json"))?;
    let lk_a = fs::read_to_string(project_a.join("package-lock.json"))?;
    let pj_b = fs::read_to_string(project_b.join("package.json"))?;
    let lk_b = fs::read_to_string(project_b.join("package-lock.json"))?;

    // A→B on project_a workspace (store already warm from both prepares).
    fs::write(project_a.join("package.json"), &pj_b)?;
    fs::write(project_a.join("package-lock.json"), &lk_b)?;
    let (outcome, dur) = time_it(|| switch_project_with_source(project_a, None, source_b));
    let outcome = outcome.map_err(anyhow::Error::msg)?;
    let (disk, files, inodes) = tree_stats(&project_a.join("node_modules"));
    rows.push(ScenarioResult {
        name: "weave-switch-a-to-b".into(),
        wall_ms: dur.as_millis(),
        disk_bytes: Some(disk),
        file_count: Some(files),
        approx_inodes: Some(inodes),
        note: Some(format!(
            "fetched={} cache_hits={}",
            outcome.prepare.fetched_artifacts, outcome.prepare.materialize.cache_hits
        )),
    });

    fs::write(project_a.join("package.json"), &pj_a)?;
    fs::write(project_a.join("package-lock.json"), &lk_a)?;
    let (outcome, dur) = time_it(|| switch_project_with_source(project_a, None, source_a));
    let outcome = outcome.map_err(anyhow::Error::msg)?;
    let (disk, files, inodes) = tree_stats(&project_a.join("node_modules"));
    rows.push(ScenarioResult {
        name: "weave-switch-b-to-a".into(),
        wall_ms: dur.as_millis(),
        disk_bytes: Some(disk),
        file_count: Some(files),
        approx_inodes: Some(inodes),
        note: Some(format!(
            "fetched={} cache_hits={}",
            outcome.prepare.fetched_artifacts, outcome.prepare.materialize.cache_hits
        )),
    });

    Ok(rows)
}

fn store_and_nm_stats(project: &Path) -> anyhow::Result<(u64, u64, u64)> {
    let store = PathBuf::from(
        ProjectConfig::load(project)
            .map_err(anyhow::Error::msg)?
            .store_path,
    );
    let unpacked = store
        .parent()
        .map(|p| p.join("unpacked"))
        .unwrap_or_else(|| store.join("unpacked"));
    Ok(trees_stats(&[
        project.join("node_modules").as_path(),
        store.as_path(),
        unpacked.as_path(),
    ]))
}

fn run_pm_ci(
    root: &Path,
    name: &str,
    args: &[&str],
    template: &Path,
    rewrites: &[(&str, &Path)],
) -> anyhow::Result<ScenarioResult> {
    let bin = if name.starts_with("pnpm") {
        "pnpm"
    } else {
        "npm"
    };
    if Command::new(bin).arg("--version").output().is_err() {
        return Ok(ScenarioResult {
            name: name.into(),
            wall_ms: 0,
            disk_bytes: None,
            file_count: None,
            approx_inodes: None,
            note: Some(format!("{bin} not available — skipped")),
        });
    }

    let dest = root.join(format!("pm-{name}"));
    // Copy package files into a fresh directory.
    fs::create_dir_all(&dest)?;
    for name in ["package.json", "package-lock.json", "README"] {
        let src = template.join(name);
        if src.is_file() {
            fs::copy(&src, dest.join(name))?;
        }
    }
    // Minimal git for tools that care.
    let _ = Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&dest)
        .status();

    let mut lock = fs::read_to_string(dest.join("package-lock.json"))?;
    for (url, path) in rewrites {
        lock = lock.replace(url, &format!("file:{}", path.display()));
    }
    fs::write(dest.join("package-lock.json"), lock)?;

    let (status, dur) = time_it(|| Command::new(bin).args(args).current_dir(&dest).status());
    let status = status?;
    let (disk, files, inodes) = tree_stats(&dest.join("node_modules"));
    Ok(ScenarioResult {
        name: name.into(),
        wall_ms: dur.as_millis(),
        disk_bytes: Some(disk),
        file_count: Some(files),
        approx_inodes: Some(inodes),
        note: Some(if status.success() {
            format!("{bin} {} with file: tarball URLs", args.join(" "))
        } else {
            format!("{bin} failed with status {status}")
        }),
    })
}

fn finish_suite(
    name: &str,
    root: PathBuf,
    rows: Vec<ScenarioResult>,
    keep_work: bool,
) -> anyhow::Result<BenchSuiteResult> {
    let summary = Some(summarize_switch_metrics(&rows));
    if keep_work {
        eprintln!("Kept work dir: {}", root.display());
    }
    std::env::remove_var("WEAVE_HOME");
    Ok(BenchSuiteResult {
        suite: name.into(),
        host: HostInfo::capture(),
        work_dir: root.display().to_string(),
        rows,
        summary,
    })
}

fn summarize_switch_metrics(rows: &[ScenarioResult]) -> String {
    let warm = rows
        .iter()
        .rev()
        .find(|r| r.name.ends_with("weave-warm") || r.name == "weave-warm")
        .map(|r| r.wall_ms);
    let sw = rows
        .iter()
        .rev()
        .find(|r| r.name.ends_with("weave-switch-a-to-b") || r.name == "weave-switch-a-to-b")
        .map(|r| r.wall_ms);
    match (warm, sw) {
        (Some(w), Some(s)) => format!("warm re-switch {w} ms; A→B switch {s} ms"),
        (Some(w), None) => format!("warm re-switch {w} ms"),
        _ => "suite complete".into(),
    }
}

fn make_work_root(keep_work: bool) -> anyhow::Result<(PathBuf, Option<tempfile::TempDir>)> {
    let td = tempfile::Builder::new().prefix("weave-bench-").tempdir()?;
    if keep_work {
        let kept = std::env::temp_dir().join(format!("weave-bench-keep-{}", std::process::id()));
        if kept.exists() {
            let _ = fs::remove_dir_all(&kept);
        }
        fs::rename(td.path(), &kept)?;
        std::mem::forget(td);
        Ok((kept, None))
    } else {
        let path = td.path().to_path_buf();
        Ok((path, Some(td)))
    }
}

/// Back-compat entry used by older call sites / tests.
#[allow(dead_code)]
pub fn run_small_suite(with_npm: bool, keep_work: bool) -> anyhow::Result<BenchSuiteResult> {
    run_suite(
        SuiteKind::Tiny,
        CompareFlags {
            npm: with_npm,
            pnpm: false,
        },
        keep_work,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_suite_produces_core_rows() {
        let result = run_suite(SuiteKind::Tiny, CompareFlags::default(), false).expect("suite");
        let names: Vec<_> = result.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "weave-cold",
                "weave-warm",
                "weave-switch-a-to-b",
                "weave-switch-b-to-a"
            ]
        );
    }

    #[test]
    fn small_scaled_suite_runs() {
        let result = run_suite(SuiteKind::Small, CompareFlags::default(), false).expect("small");
        assert_eq!(result.suite, "small");
        assert!(result.rows.iter().any(|r| r.name == "weave-cold"));
        assert!(result.rows.iter().any(|r| r.name == "weave-switch-a-to-b"));
    }
}
