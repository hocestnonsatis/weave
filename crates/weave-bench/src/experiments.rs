//! Dependency divergence and materialization-pressure experiments.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use weave_engine::{init_project, switch_project_with_source, ProjectConfig};
use weave_lockfile::parse_lockfile;

use crate::analyze::{artifact_set, overlap, OverlapReport};
use crate::fixture::{write_scaled_project, BenchEnv, ScaleSpec, ScaledPackages};
use crate::measure::{time_it, tree_stats, trees_stats, ScenarioResult};

/// Target shared-artifact fractions for synthetic divergence.
pub const DIVERGENCE_TARGETS: &[f64] = &[0.95, 0.75, 0.50, 0.25, 0.0];

#[derive(Debug, Clone, Serialize)]
pub struct DivergenceRow {
    pub label: String,
    pub target_shared_fraction: f64,
    pub measured_overlap: OverlapReport,
    pub weave_cold_ms: Option<u128>,
    pub weave_warm_ms: Option<u128>,
    pub weave_switch_a_to_b_ms: Option<u128>,
    pub weave_switch_b_to_a_ms: Option<u128>,
    pub cold_hardlinks: Option<usize>,
    pub cold_copies: Option<usize>,
    pub note: String,
}

/// Build scaled A/B sets aiming for `shared_fraction` of A's artifacts shared with B.
pub fn run_synthetic_divergence(keep_work: bool) -> anyhow::Result<Vec<DivergenceRow>> {
    let mut rows = Vec::new();
    for &target in DIVERGENCE_TARGETS {
        let total = 40usize;
        let shared = ((total as f64) * target).round() as usize;
        let a_only = total.saturating_sub(shared);
        let b_unique = if target == 0.0 { total } else { a_only.max(1) };
        let spec = ScaleSpec {
            name: "divergence",
            package_count: total,
            extra_files_per_pkg: 3,
            shared_count: shared,
            b_unique,
        };
        let row = run_one_scaled_divergence(spec, target, keep_work && target == 0.5)?;
        rows.push(row);
    }
    Ok(rows)
}

fn run_one_scaled_divergence(
    spec: ScaleSpec,
    target: f64,
    keep_work: bool,
) -> anyhow::Result<DivergenceRow> {
    let td = tempfile::Builder::new().prefix("weave-div-").tempdir()?;
    let root = td.path().to_path_buf();
    let weave_home = root.join("weave-home");
    let tarball_dir = root.join("tarballs");
    fs::create_dir_all(&weave_home)?;
    std::env::set_var("WEAVE_HOME", &weave_home);

    let pkgs = ScaledPackages::create(&tarball_dir, spec, false)?;
    let project_a = root.join("a");
    let project_b = root.join("b");
    write_scaled_project(&project_a, BenchEnv::A, &pkgs, false)?;
    write_scaled_project(&project_b, BenchEnv::B, &pkgs, false)?;

    let ga = parse_lockfile(&project_a.join("package-lock.json")).map_err(anyhow::Error::msg)?;
    let gb = parse_lockfile(&project_b.join("package-lock.json")).map_err(anyhow::Error::msg)?;
    let measured = overlap(&artifact_set(&ga), &artifact_set(&gb));

    let source_a = pkgs.source_a(&tarball_dir);
    let source_b = pkgs.source_b(&tarball_dir);

    init_project(&project_a).map_err(anyhow::Error::msg)?;
    let (out_cold, cold_ms) = time_it(|| switch_project_with_source(&project_a, None, &source_a));
    let out_cold = out_cold.map_err(anyhow::Error::msg)?;
    let (out_warm, warm_ms) = time_it(|| switch_project_with_source(&project_a, None, &source_a));
    let _ = out_warm.map_err(anyhow::Error::msg)?;

    init_project(&project_b).map_err(anyhow::Error::msg)?;
    switch_project_with_source(&project_b, None, &source_b).map_err(anyhow::Error::msg)?;

    let pj_a = fs::read_to_string(project_a.join("package.json"))?;
    let lk_a = fs::read_to_string(project_a.join("package-lock.json"))?;
    let pj_b = fs::read_to_string(project_b.join("package.json"))?;
    let lk_b = fs::read_to_string(project_b.join("package-lock.json"))?;

    fs::write(project_a.join("package.json"), &pj_b)?;
    fs::write(project_a.join("package-lock.json"), &lk_b)?;
    let (ab_out, ab_ms) = time_it(|| switch_project_with_source(&project_a, None, &source_b));
    let ab_out = ab_out.map_err(anyhow::Error::msg)?;

    fs::write(project_a.join("package.json"), &pj_a)?;
    fs::write(project_a.join("package-lock.json"), &lk_a)?;
    let (ba_out, ba_ms) = time_it(|| switch_project_with_source(&project_a, None, &source_a));
    let _ = ba_out.map_err(anyhow::Error::msg)?;

    if keep_work {
        eprintln!("Kept divergence work: {}", root.display());
        std::mem::forget(td);
    }

    std::env::remove_var("WEAVE_HOME");
    Ok(DivergenceRow {
        label: format!("synthetic-shared-{:.0}%", target * 100.0),
        target_shared_fraction: target,
        measured_overlap: measured.clone(),
        weave_cold_ms: Some(cold_ms.as_millis()),
        weave_warm_ms: Some(warm_ms.as_millis()),
        weave_switch_a_to_b_ms: Some(ab_ms.as_millis()),
        weave_switch_b_to_a_ms: Some(ba_ms.as_millis()),
        cold_hardlinks: Some(out_cold.prepare.materialize.hardlinked_files),
        cold_copies: Some(out_cold.prepare.materialize.copied_files),
        note: format!(
            "shared={} only_a={} only_b={} jaccard={:.3}; ab_cache_hits={}",
            measured.shared,
            measured.only_a,
            measured.only_b,
            measured.jaccard,
            ab_out.prepare.materialize.cache_hits
        ),
    })
}

/// Real corpus pair overlap (analysis only — no tarball fetch).
pub fn real_pair_overlap(lock_a: &Path, lock_b: &Path) -> anyhow::Result<(OverlapReport, String)> {
    let ga = parse_lockfile(lock_a).map_err(anyhow::Error::msg)?;
    let gb = parse_lockfile(lock_b).map_err(anyhow::Error::msg)?;
    let report = overlap(&artifact_set(&ga), &artifact_set(&gb));
    Ok((
        report,
        format!("{} vs {}", lock_a.display(), lock_b.display()),
    ))
}

#[derive(Debug, Clone, Serialize)]
pub struct MaterializePressureRow {
    pub label: String,
    pub packages: usize,
    pub files_per_pkg: usize,
    pub wall_ms: u128,
    pub hardlinks: usize,
    pub copies: usize,
    pub disk_bytes_nm: u64,
    pub disk_bytes_store_unpacked: u64,
    pub inodes_nm: u64,
    pub force_copy: bool,
    pub note: String,
}

/// Materialization pressure across package counts; optional force-copy via native flags.
pub fn run_materialize_pressure(keep_work: bool) -> anyhow::Result<Vec<MaterializePressureRow>> {
    let mut rows = Vec::new();
    for &(count, files) in &[(25usize, 3usize), (80, 4), (150, 4), (250, 5)] {
        rows.push(pressure_one(count, files, false, keep_work && count == 80)?);
        // Prefer-copy stress: mark packages native in scaled set via with_native on small subset
    }
    // Force-copy path: small native-heavy set
    rows.push(pressure_native_copy(40, 3, keep_work)?);
    Ok(rows)
}

fn pressure_one(
    count: usize,
    files: usize,
    _force_copy: bool,
    keep_work: bool,
) -> anyhow::Result<MaterializePressureRow> {
    let td = tempfile::Builder::new().prefix("weave-press-").tempdir()?;
    let root = td.path().to_path_buf();
    let weave_home = root.join("weave-home");
    let tarball_dir = root.join("tarballs");
    fs::create_dir_all(&weave_home)?;
    std::env::set_var("WEAVE_HOME", &weave_home);

    let spec = ScaleSpec {
        name: "pressure",
        package_count: count,
        extra_files_per_pkg: files,
        shared_count: count,
        b_unique: 0,
    };
    let pkgs = ScaledPackages::create(&tarball_dir, spec, false)?;
    let project = root.join("project");
    write_scaled_project(&project, BenchEnv::A, &pkgs, false)?;
    let source = pkgs.source_a(&tarball_dir);
    init_project(&project).map_err(anyhow::Error::msg)?;
    let (outcome, dur) = time_it(|| switch_project_with_source(&project, None, &source));
    let outcome = outcome.map_err(anyhow::Error::msg)?;

    let nm = tree_stats(&project.join("node_modules"));
    let store = PathBuf::from(
        ProjectConfig::load(&project)
            .map_err(anyhow::Error::msg)?
            .store_path,
    );
    let unpacked = store
        .parent()
        .map(|p| p.join("unpacked"))
        .unwrap_or_else(|| store.join("unpacked"));
    let store_stats = trees_stats(&[store.as_path(), unpacked.as_path()]);

    if keep_work {
        eprintln!("Kept pressure work: {}", root.display());
        std::mem::forget(td);
    }
    std::env::remove_var("WEAVE_HOME");

    Ok(MaterializePressureRow {
        label: format!("pkgs={count}/files={files}"),
        packages: count,
        files_per_pkg: files,
        wall_ms: dur.as_millis(),
        hardlinks: outcome.prepare.materialize.hardlinked_files,
        copies: outcome.prepare.materialize.copied_files,
        disk_bytes_nm: nm.0,
        disk_bytes_store_unpacked: store_stats.0,
        inodes_nm: nm.2,
        force_copy: false,
        note: format!(
            "cache_misses={} fetched={}",
            outcome.prepare.materialize.cache_misses, outcome.prepare.fetched_artifacts
        ),
    })
}

fn pressure_native_copy(
    count: usize,
    files: usize,
    keep_work: bool,
) -> anyhow::Result<MaterializePressureRow> {
    let td = tempfile::Builder::new()
        .prefix("weave-press-n-")
        .tempdir()?;
    let root = td.path().to_path_buf();
    let weave_home = root.join("weave-home");
    let tarball_dir = root.join("tarballs");
    fs::create_dir_all(&weave_home)?;
    std::env::set_var("WEAVE_HOME", &weave_home);

    let spec = ScaleSpec {
        name: "pressure-native",
        package_count: count,
        extra_files_per_pkg: files,
        shared_count: count.saturating_sub(1),
        b_unique: 0,
    };
    let pkgs = ScaledPackages::create(&tarball_dir, spec, true)?;
    let project = root.join("project");
    write_scaled_project(&project, BenchEnv::A, &pkgs, true)?;
    let source = pkgs.source_a(&tarball_dir);
    init_project(&project).map_err(anyhow::Error::msg)?;
    let (outcome, dur) = time_it(|| switch_project_with_source(&project, None, &source));
    let outcome = outcome.map_err(anyhow::Error::msg)?;
    let nm = tree_stats(&project.join("node_modules"));

    if keep_work {
        std::mem::forget(td);
    }
    std::env::remove_var("WEAVE_HOME");

    Ok(MaterializePressureRow {
        label: format!("pkgs={count}+native-addon"),
        packages: count + 1,
        files_per_pkg: files,
        wall_ms: dur.as_millis(),
        hardlinks: outcome.prepare.materialize.hardlinked_files,
        copies: outcome.prepare.materialize.copied_files,
        disk_bytes_nm: nm.0,
        disk_bytes_store_unpacked: 0,
        inodes_nm: nm.2,
        force_copy: true,
        note: "native-addon prefer_copy; remaining packages may hardlink".into(),
    })
}

/// Scenario results alias for report embedding.
#[allow(dead_code)]
pub fn divergence_as_scenarios(rows: &[DivergenceRow]) -> Vec<ScenarioResult> {
    rows.iter()
        .map(|r| ScenarioResult {
            name: r.label.clone(),
            wall_ms: r.weave_switch_a_to_b_ms.unwrap_or(0),
            disk_bytes: None,
            file_count: None,
            approx_inodes: None,
            note: Some(format!(
                "target={:.0}% measured_shared_of_a={:.1}% cold={}ms warm={}ms ba={}ms | {}",
                r.target_shared_fraction * 100.0,
                r.measured_overlap.shared_fraction_of_a * 100.0,
                r.weave_cold_ms.unwrap_or(0),
                r.weave_warm_ms.unwrap_or(0),
                r.weave_switch_b_to_a_ms.unwrap_or(0),
                r.note
            )),
        })
        .collect()
}

/// Collect integrity fingerprints from a lockfile for dedup estimates.
#[allow(dead_code)]
pub fn lockfile_artifact_ids(path: &Path) -> anyhow::Result<BTreeSet<String>> {
    let g = parse_lockfile(path).map_err(anyhow::Error::msg)?;
    Ok(artifact_set(&g))
}
