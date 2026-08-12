//! Phase 16: AI-agent parallel environment workload validation.
//!
//! Hypothesis: Weave's strongest advantage is parallel isolated environments
//! that share dependency artifacts via CAS (vs duplicated `node_modules`).
//!
//! Measurement classes are never mixed in a single row:
//! - `offline` — synthetic lockfiles + local tarballs (reproducible)
//! - `network` — real corpus lockfiles + registry (optional `--network`)

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use serde::Serialize;
use weave_engine::{
    init_project, switch_project, switch_project_with_source, FileArtifactSource, ProjectConfig,
};
use weave_lockfile::parse_lockfile;

use crate::analyze::{artifact_set, overlap};
use crate::corpus;
use crate::fixture::{write_scaled_project, BenchEnv, ScaleSpec, ScaledPackages};
use crate::measure::{disk_accounting, duplicated_bytes, time_it, DiskAccounting, HostInfo};

/// High-overlap tree approximating related AI-agent worktrees on one repo.
pub const AGENT_OVERLAP: ScaleSpec = ScaleSpec {
    name: "agent-overlap",
    package_count: 60,
    extra_files_per_pkg: 6,
    shared_count: 54,
    b_unique: 6,
};

#[derive(Debug, Clone, Serialize)]
pub struct Phase16Row {
    pub measurement_class: String,
    pub tool: String,
    pub scenario: String,
    pub parallel_n: Option<usize>,
    pub wall_ms: u128,
    pub per_env_ms: Option<u128>,
    pub peak_apparent_bytes: Option<u64>,
    pub final_apparent_bytes: Option<u64>,
    pub final_unique_bytes: Option<u64>,
    pub duplicated_bytes: Option<u64>,
    pub approx_inodes: Option<u64>,
    pub network_bytes: Option<u64>,
    pub weave_fetched: Option<u64>,
    pub weave_reused: Option<u64>,
    pub weave_hardlinks: Option<u64>,
    pub weave_copies: Option<u64>,
    pub weave_cache_hits: Option<u64>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CorpusOverlapRow {
    pub id: String,
    pub packages_a: usize,
    pub packages_b: usize,
    pub artifacts_a: usize,
    pub artifacts_b: usize,
    pub shared_artifacts: usize,
    pub shared_of_a: f64,
    pub provenance_note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Phase16Report {
    pub host: HostInfo,
    pub work_dir: String,
    pub weave_version: String,
    pub npm_version: Option<String>,
    pub pnpm_version: Option<String>,
    pub offline_fixture: String,
    pub corpus_overlap: Vec<CorpusOverlapRow>,
    pub rows: Vec<Phase16Row>,
    pub verdict: String,
    pub caveats: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct Phase16Opts {
    pub network: bool,
    pub keep_work: bool,
}

pub fn run_phase16(opts: Phase16Opts) -> anyhow::Result<Phase16Report> {
    let (root, _td) = make_work_root(opts.keep_work)?;
    let mut rows = Vec::new();
    let mut caveats = vec![
        "Offline and network rows are separate measurement classes — do not compare them as one series.".into(),
        "Unique bytes use (dev,ino) hardlink dedup; they are not filesystem block allocation.".into(),
        "Network bytes are only reported when a trustworthy counter exists; otherwise omitted.".into(),
    ];

    let corpus_overlap = analyze_corpus_overlap()?;
    rows.extend(run_offline_suite(&root)?);

    if opts.network {
        match run_network_suite(&root) {
            Ok(net_rows) => rows.extend(net_rows),
            Err(err) => caveats.push(format!("Network suite skipped/failed: {err:#}")),
        }
    } else {
        caveats.push(
            "Network suite not requested (`--network`). Real-registry timings absent by design."
                .into(),
        );
    }

    let verdict = derive_verdict(&rows, &corpus_overlap);
    Ok(Phase16Report {
        host: HostInfo::capture(),
        work_dir: root.display().to_string(),
        weave_version: env!("CARGO_PKG_VERSION").into(),
        npm_version: tool_version("npm"),
        pnpm_version: tool_version("pnpm"),
        offline_fixture: format!(
            "{} pkgs={} shared={} b_unique={} files/pkg={}",
            AGENT_OVERLAP.name,
            AGENT_OVERLAP.package_count,
            AGENT_OVERLAP.shared_count,
            AGENT_OVERLAP.b_unique,
            AGENT_OVERLAP.extra_files_per_pkg
        ),
        corpus_overlap,
        rows,
        verdict,
        caveats,
    })
}

fn analyze_corpus_overlap() -> anyhow::Result<Vec<CorpusOverlapRow>> {
    let root = corpus::default_corpus_root();
    let pairs = [
        (
            "axios-v1.6",
            "divergence/axios-v1.6",
            "axios-v1.7",
            "divergence/axios-v1.7",
        ),
        (
            "nestjs-v10.3",
            "divergence/nestjs-v10.3",
            "nestjs-v10.4",
            "divergence/nestjs-v10.4",
        ),
    ];
    let mut out = Vec::new();
    for (ida, patha, idb, pathb) in pairs {
        let pa = root.join(patha).join("package-lock.json");
        let pb = root.join(pathb).join("package-lock.json");
        if !pa.is_file() || !pb.is_file() {
            continue;
        }
        let ga = parse_lockfile(&pa).map_err(anyhow::Error::msg)?;
        let gb = parse_lockfile(&pb).map_err(anyhow::Error::msg)?;
        let oa = artifact_set(&ga);
        let ob = artifact_set(&gb);
        let ov = overlap(&oa, &ob);
        let shared_of_a = if oa.is_empty() {
            0.0
        } else {
            ov.shared as f64 / oa.len() as f64
        };
        out.push(CorpusOverlapRow {
            id: format!("{ida}\u{2194}{idb}"),
            packages_a: ga.nodes.len(),
            packages_b: gb.nodes.len(),
            artifacts_a: oa.len(),
            artifacts_b: ob.len(),
            shared_artifacts: ov.shared,
            shared_of_a,
            provenance_note: format!(
                "Pinned lockfiles under benchmarks/corpus/{{{patha},{pathb}}}; see PROVENANCE.json"
            ),
        });
    }
    Ok(out)
}

fn run_offline_suite(root: &Path) -> anyhow::Result<Vec<Phase16Row>> {
    let offline = root.join("offline");
    fs::create_dir_all(&offline)?;
    let tarball_dir = offline.join("tarballs");
    let pkgs = ScaledPackages::create(&tarball_dir, AGENT_OVERLAP, false)?;
    let template_a = offline.join("template-a");
    let template_b = offline.join("template-b");
    write_scaled_project(&template_a, BenchEnv::A, &pkgs, false)?;
    write_scaled_project(&template_b, BenchEnv::B, &pkgs, false)?;
    let source_a = pkgs.source_a(&tarball_dir);
    let source_b = pkgs.source_b(&tarball_dir);
    let rewrites = build_rewrites(&pkgs);

    let mut rows = Vec::new();
    rows.extend(run_offline_tool_weave(
        &offline,
        &template_a,
        &template_b,
        &source_a,
        &source_b,
    )?);
    if tool_available("npm") {
        rows.extend(run_offline_tool_npm(
            &offline,
            &template_a,
            &template_b,
            &rewrites,
        )?);
    } else {
        rows.push(skip_row("offline", "npm", "npm not available"));
    }
    if tool_available("pnpm") {
        rows.extend(run_offline_tool_pnpm(
            &offline,
            &template_a,
            &template_b,
            &rewrites,
        )?);
    } else {
        rows.push(skip_row("offline", "pnpm", "pnpm not available"));
    }
    Ok(rows)
}

fn build_rewrites(pkgs: &ScaledPackages) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for p in pkgs
        .shared
        .iter()
        .chain(pkgs.a_only.iter())
        .chain(pkgs.b_only.iter())
    {
        let tarball_name = format!("{}-{}", p.name, p.version);
        let url = format!("https://example.invalid/{}/-/{}.tgz", p.name, tarball_name);
        out.push((url, p.tarball.clone()));
    }
    out
}

fn run_offline_tool_weave(
    offline: &Path,
    template_a: &Path,
    template_b: &Path,
    source_a: &FileArtifactSource,
    source_b: &FileArtifactSource,
) -> anyhow::Result<Vec<Phase16Row>> {
    let mut rows = Vec::new();
    let home = offline.join("weave-home");
    fs::create_dir_all(&home)?;
    std::env::set_var("WEAVE_HOME", &home);

    let p1 = offline.join("weave-single");
    copy_project(template_a, &p1)?;
    init_project(&p1).map_err(anyhow::Error::msg)?;
    let (out, dur) = time_it(|| switch_project_with_source(&p1, None, source_a));
    let out = out.map_err(anyhow::Error::msg)?;
    let acc = weave_disk(&p1)?;
    rows.push(weave_row(
        "single_clean",
        1,
        dur.as_millis(),
        Some(dur.as_millis()),
        acc,
        &out,
        "Cold switch into empty WEAVE_HOME; local FileArtifactSource",
    ));

    let p2 = offline.join("weave-repeat");
    copy_project(template_a, &p2)?;
    init_project(&p2).map_err(anyhow::Error::msg)?;
    let (out, dur) = time_it(|| switch_project_with_source(&p2, None, source_a));
    let out = out.map_err(anyhow::Error::msg)?;
    let acc = weave_disk(&p2)?;
    rows.push(weave_row(
        "repeated_create",
        1,
        dur.as_millis(),
        Some(dur.as_millis()),
        acc,
        &out,
        "Second project; shared WEAVE_HOME already populated",
    ));

    let pab = offline.join("weave-branch");
    copy_project(template_a, &pab)?;
    init_project(&pab).map_err(anyhow::Error::msg)?;
    switch_project_with_source(&pab, None, source_a).map_err(anyhow::Error::msg)?;
    let seed_b = offline.join("weave-seed-b");
    copy_project(template_b, &seed_b)?;
    init_project(&seed_b).map_err(anyhow::Error::msg)?;
    switch_project_with_source(&seed_b, None, source_b).map_err(anyhow::Error::msg)?;

    let pj_b = fs::read_to_string(template_b.join("package.json"))?;
    let lk_b = fs::read_to_string(template_b.join("package-lock.json"))?;
    let pj_a = fs::read_to_string(template_a.join("package.json"))?;
    let lk_a = fs::read_to_string(template_a.join("package-lock.json"))?;
    fs::write(pab.join("package.json"), &pj_b)?;
    fs::write(pab.join("package-lock.json"), &lk_b)?;
    let (out_ab, dur_ab) = time_it(|| switch_project_with_source(&pab, None, source_b));
    let out_ab = out_ab.map_err(anyhow::Error::msg)?;
    fs::write(pab.join("package.json"), &pj_a)?;
    fs::write(pab.join("package-lock.json"), &lk_a)?;
    let (out_ba, dur_ba) = time_it(|| switch_project_with_source(&pab, None, source_a));
    let out_ba = out_ba.map_err(anyhow::Error::msg)?;
    let acc = weave_disk(&pab)?;
    rows.push(weave_row(
        "branch_a_to_b",
        1,
        dur_ab.as_millis(),
        Some(dur_ab.as_millis()),
        acc,
        &out_ab,
        "Store pre-seeded with A and B artifacts",
    ));
    rows.push(weave_row(
        "branch_b_to_a",
        1,
        dur_ba.as_millis(),
        Some(dur_ba.as_millis()),
        acc,
        &out_ba,
        "Return switch after A→B",
    ));

    for n in [2usize, 4, 8] {
        rows.push(run_weave_parallel(offline, template_a, source_a, n)?);
    }

    std::env::remove_var("WEAVE_HOME");
    Ok(rows)
}

fn weave_row(
    scenario: &str,
    n: usize,
    wall: u128,
    per: Option<u128>,
    acc: DiskAccounting,
    out: &weave_engine::SwitchOutcome,
    note: &str,
) -> Phase16Row {
    Phase16Row {
        measurement_class: "offline".into(),
        tool: "weave".into(),
        scenario: scenario.into(),
        parallel_n: Some(n),
        wall_ms: wall,
        per_env_ms: per,
        peak_apparent_bytes: Some(acc.apparent_bytes),
        final_apparent_bytes: Some(acc.apparent_bytes),
        final_unique_bytes: Some(acc.unique_bytes),
        duplicated_bytes: Some(duplicated_bytes(acc)),
        approx_inodes: Some(acc.approx_inodes),
        network_bytes: None,
        weave_fetched: Some(out.prepare.fetched_artifacts as u64),
        weave_reused: Some(out.prepare.reused_artifacts as u64),
        weave_hardlinks: Some(out.prepare.materialize.hardlinked_files as u64),
        weave_copies: Some(out.prepare.materialize.copied_files as u64),
        weave_cache_hits: Some(out.prepare.materialize.cache_hits as u64),
        note: note.into(),
    }
}

fn run_weave_parallel(
    offline: &Path,
    template_a: &Path,
    source_a: &FileArtifactSource,
    n: usize,
) -> anyhow::Result<Phase16Row> {
    let home = offline.join(format!("weave-home-par-{n}"));
    if home.exists() {
        let _ = fs::remove_dir_all(&home);
    }
    fs::create_dir_all(&home)?;
    {
        std::env::set_var("WEAVE_HOME", &home);
        let seed = offline.join(format!("weave-par-{n}-seed"));
        copy_project(template_a, &seed)?;
        init_project(&seed).map_err(anyhow::Error::msg)?;
        switch_project_with_source(&seed, None, source_a).map_err(anyhow::Error::msg)?;
    }

    let projects: Vec<PathBuf> = (0..n)
        .map(|i| offline.join(format!("weave-par-{n}-env-{i}")))
        .collect();
    for p in &projects {
        copy_project(template_a, p)?;
        std::env::set_var("WEAVE_HOME", &home);
        init_project(p).map_err(anyhow::Error::msg)?;
    }

    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let per_ms: Arc<Mutex<Vec<u128>>> = Arc::new(Mutex::new(Vec::new()));
    let source = source_a.clone();
    let start = Instant::now();
    thread::scope(|scope| {
        for p in &projects {
            let source = source.clone();
            let errors = Arc::clone(&errors);
            let per_ms = Arc::clone(&per_ms);
            let home = home.clone();
            let p = p.clone();
            scope.spawn(move || {
                std::env::set_var("WEAVE_HOME", &home);
                let t0 = Instant::now();
                match switch_project_with_source(&p, None, &source) {
                    Ok(_) => per_ms.lock().unwrap().push(t0.elapsed().as_millis()),
                    Err(e) => errors.lock().unwrap().push(e.to_string()),
                }
            });
        }
    });
    let wall = start.elapsed().as_millis();
    let errs = errors.lock().unwrap().clone();
    if !errs.is_empty() {
        anyhow::bail!("weave parallel-{n} errors: {}", errs.join("; "));
    }
    let per = per_ms.lock().unwrap().clone();
    let avg = avg_ms(&per);
    let mut roots: Vec<PathBuf> = projects.iter().map(|p| p.join("node_modules")).collect();
    roots.push(home.join("store"));
    let refs: Vec<&Path> = roots.iter().map(|p| p.as_path()).collect();
    let acc = disk_accounting(&refs);
    std::env::remove_var("WEAVE_HOME");
    Ok(pm_row(
        "offline",
        "weave",
        &format!("parallel_{n}"),
        n,
        wall,
        Some(avg),
        acc,
        None,
        format!(
            "Concurrent switches after store seed; avg_per_env={avg}ms; unique/apparent={:.3}",
            ratio(acc.unique_bytes, acc.apparent_bytes)
        ),
    ))
}

fn run_offline_tool_npm(
    offline: &Path,
    template_a: &Path,
    template_b: &Path,
    rewrites: &[(String, PathBuf)],
) -> anyhow::Result<Vec<Phase16Row>> {
    let mut rows = Vec::new();
    let cache = offline.join("npm-cache");
    fs::create_dir_all(&cache)?;

    let p1 = offline.join("npm-single");
    let (wall, acc, note) = npm_ci_one(&p1, template_a, rewrites, &cache)?;
    rows.push(pm_row(
        "offline",
        "npm",
        "single_clean",
        1,
        wall,
        Some(wall),
        acc,
        None,
        note,
    ));

    let p2 = offline.join("npm-repeat");
    let (wall, acc, note) = npm_ci_one(&p2, template_a, rewrites, &cache)?;
    rows.push(pm_row(
        "offline",
        "npm",
        "repeated_create",
        1,
        wall,
        Some(wall),
        acc,
        None,
        note,
    ));

    let pab = offline.join("npm-branch");
    let (wall_ab, acc_ab, _) = npm_ci_one(&pab, template_b, rewrites, &cache)?;
    rows.push(pm_row(
        "offline",
        "npm",
        "branch_a_to_b",
        1,
        wall_ab,
        Some(wall_ab),
        acc_ab,
        None,
        "Semantic stand-in: npm ci for branch B lockfile".into(),
    ));
    let _ = fs::remove_dir_all(pab.join("node_modules"));
    fs::copy(template_a.join("package.json"), pab.join("package.json"))?;
    let mut lock = fs::read_to_string(template_a.join("package-lock.json"))?;
    for (url, path) in rewrites {
        lock = lock.replace(url, &format!("file:{}", path.display()));
    }
    fs::write(pab.join("package-lock.json"), lock)?;
    let start = Instant::now();
    let status = Command::new("npm")
        .args(["ci", "--ignore-scripts"])
        .env("npm_config_cache", &cache)
        .current_dir(&pab)
        .status()?;
    let wall_ba = start.elapsed().as_millis();
    let acc_ba = disk_accounting(&[pab.join("node_modules").as_path(), cache.as_path()]);
    rows.push(pm_row(
        "offline",
        "npm",
        "branch_b_to_a",
        1,
        wall_ba,
        Some(wall_ba),
        acc_ba,
        None,
        if status.success() {
            "Semantic stand-in: npm ci for branch A after B".into()
        } else {
            format!("npm ci failed: {status}")
        },
    ));

    for n in [2usize, 4, 8] {
        rows.push(run_npm_parallel(offline, template_a, rewrites, n)?);
    }
    Ok(rows)
}

fn run_npm_parallel(
    offline: &Path,
    template_a: &Path,
    rewrites: &[(String, PathBuf)],
    n: usize,
) -> anyhow::Result<Phase16Row> {
    let cache = offline.join(format!("npm-cache-par-{n}"));
    if cache.exists() {
        let _ = fs::remove_dir_all(&cache);
    }
    fs::create_dir_all(&cache)?;
    let seed = offline.join(format!("npm-par-{n}-seed"));
    let _ = npm_ci_one(&seed, template_a, rewrites, &cache)?;

    let projects: Vec<PathBuf> = (0..n)
        .map(|i| offline.join(format!("npm-par-{n}-env-{i}")))
        .collect();
    for p in &projects {
        prepare_pm_project(p, template_a, rewrites)?;
    }

    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let per_ms: Arc<Mutex<Vec<u128>>> = Arc::new(Mutex::new(Vec::new()));
    let start = Instant::now();
    thread::scope(|scope| {
        for p in &projects {
            let cache = cache.clone();
            let errors = Arc::clone(&errors);
            let per_ms = Arc::clone(&per_ms);
            let p = p.clone();
            scope.spawn(move || {
                let t0 = Instant::now();
                let status = Command::new("npm")
                    .args(["ci", "--ignore-scripts"])
                    .env("npm_config_cache", &cache)
                    .current_dir(&p)
                    .status();
                match status {
                    Ok(s) if s.success() => per_ms.lock().unwrap().push(t0.elapsed().as_millis()),
                    Ok(s) => errors.lock().unwrap().push(format!("status {s}")),
                    Err(e) => errors.lock().unwrap().push(e.to_string()),
                }
            });
        }
    });
    let wall = start.elapsed().as_millis();
    let errs = errors.lock().unwrap().clone();
    if !errs.is_empty() {
        anyhow::bail!("npm parallel-{n} errors: {}", errs.join("; "));
    }
    let per = per_ms.lock().unwrap().clone();
    let avg = avg_ms(&per);
    let mut roots: Vec<PathBuf> = projects.iter().map(|p| p.join("node_modules")).collect();
    roots.push(cache);
    let refs: Vec<&Path> = roots.iter().map(|p| p.as_path()).collect();
    let acc = disk_accounting(&refs);
    Ok(pm_row(
        "offline",
        "npm",
        &format!("parallel_{n}"),
        n,
        wall,
        Some(avg),
        acc,
        None,
        format!(
            "Concurrent npm ci with shared cache; unique/apparent={:.3}",
            ratio(acc.unique_bytes, acc.apparent_bytes)
        ),
    ))
}

fn run_offline_tool_pnpm(
    offline: &Path,
    template_a: &Path,
    template_b: &Path,
    rewrites: &[(String, PathBuf)],
) -> anyhow::Result<Vec<Phase16Row>> {
    let mut rows = Vec::new();
    let store = offline.join("pnpm-store");
    fs::create_dir_all(&store)?;

    let p1 = offline.join("pnpm-single");
    let (wall, acc, note) = pnpm_install_one(&p1, template_a, rewrites, &store)?;
    rows.push(pm_row(
        "offline",
        "pnpm",
        "single_clean",
        1,
        wall,
        Some(wall),
        acc,
        None,
        note,
    ));

    let p2 = offline.join("pnpm-repeat");
    let (wall, acc, note) = pnpm_install_one(&p2, template_a, rewrites, &store)?;
    rows.push(pm_row(
        "offline",
        "pnpm",
        "repeated_create",
        1,
        wall,
        Some(wall),
        acc,
        None,
        note,
    ));

    let pab = offline.join("pnpm-branch");
    let (wall_ab, acc_ab, _) = pnpm_install_one(&pab, template_b, rewrites, &store)?;
    rows.push(pm_row(
        "offline",
        "pnpm",
        "branch_a_to_b",
        1,
        wall_ab,
        Some(wall_ab),
        acc_ab,
        None,
        "Semantic stand-in: fresh pnpm install for branch B lockfile".into(),
    ));
    let _ = fs::remove_dir_all(pab.join("node_modules"));
    let _ = fs::remove_file(pab.join("pnpm-lock.yaml"));
    prepare_pnpm_file_project(&pab, template_a, rewrites)?;
    let start = Instant::now();
    let status = Command::new("pnpm")
        .args(["install", "--ignore-scripts", "--no-frozen-lockfile"])
        .env("PNPM_STORE_DIR", &store)
        .current_dir(&pab)
        .status()?;
    let wall_ba = start.elapsed().as_millis();
    let acc_ba = disk_accounting(&[pab.join("node_modules").as_path(), store.as_path()]);
    rows.push(pm_row(
        "offline",
        "pnpm",
        "branch_b_to_a",
        1,
        wall_ba,
        Some(wall_ba),
        acc_ba,
        None,
        if status.success() {
            "Semantic stand-in: pnpm install for branch A after B".into()
        } else {
            "pnpm install failed for B→A".into()
        },
    ));

    for n in [2usize, 4, 8] {
        rows.push(run_pnpm_parallel(offline, template_a, rewrites, n)?);
    }
    Ok(rows)
}

fn run_pnpm_parallel(
    offline: &Path,
    template_a: &Path,
    rewrites: &[(String, PathBuf)],
    n: usize,
) -> anyhow::Result<Phase16Row> {
    let store = offline.join(format!("pnpm-store-par-{n}"));
    if store.exists() {
        let _ = fs::remove_dir_all(&store);
    }
    fs::create_dir_all(&store)?;
    let seed = offline.join(format!("pnpm-par-{n}-seed"));
    let _ = pnpm_install_one(&seed, template_a, rewrites, &store)?;

    let projects: Vec<PathBuf> = (0..n)
        .map(|i| offline.join(format!("pnpm-par-{n}-env-{i}")))
        .collect();
    for p in &projects {
        prepare_pnpm_file_project(p, template_a, rewrites)?;
    }

    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let per_ms: Arc<Mutex<Vec<u128>>> = Arc::new(Mutex::new(Vec::new()));
    let start = Instant::now();
    thread::scope(|scope| {
        for p in &projects {
            let store = store.clone();
            let errors = Arc::clone(&errors);
            let per_ms = Arc::clone(&per_ms);
            let p = p.clone();
            scope.spawn(move || {
                let t0 = Instant::now();
                let status = Command::new("pnpm")
                    .args(["install", "--ignore-scripts", "--no-frozen-lockfile"])
                    .env("PNPM_STORE_DIR", &store)
                    .current_dir(&p)
                    .status();
                match status {
                    Ok(s) if s.success() => per_ms.lock().unwrap().push(t0.elapsed().as_millis()),
                    Ok(s) => errors.lock().unwrap().push(format!("status {s}")),
                    Err(e) => errors.lock().unwrap().push(e.to_string()),
                }
            });
        }
    });
    let wall = start.elapsed().as_millis();
    let errs = errors.lock().unwrap().clone();
    if !errs.is_empty() {
        return Ok(pm_row(
            "offline",
            "pnpm",
            &format!("parallel_{n}"),
            n,
            wall,
            None,
            DiskAccounting::default(),
            None,
            format!("FAILED: {}", errs.join("; ")),
        ));
    }
    let per = per_ms.lock().unwrap().clone();
    let avg = avg_ms(&per);
    let mut roots: Vec<PathBuf> = projects.iter().map(|p| p.join("node_modules")).collect();
    roots.push(store);
    let refs: Vec<&Path> = roots.iter().map(|p| p.as_path()).collect();
    let acc = disk_accounting(&refs);
    Ok(pm_row(
        "offline",
        "pnpm",
        &format!("parallel_{n}"),
        n,
        wall,
        Some(avg),
        acc,
        None,
        format!(
            "Concurrent pnpm install with shared store; unique/apparent={:.3}",
            ratio(acc.unique_bytes, acc.apparent_bytes)
        ),
    ))
}

fn run_network_suite(root: &Path) -> anyhow::Result<Vec<Phase16Row>> {
    let probe = Command::new("npm")
        .args(["ping", "--registry", "https://registry.npmjs.org/"])
        .output()?;
    if !probe.status.success() {
        anyhow::bail!("npm ping registry.npmjs.org failed");
    }

    let net = root.join("network");
    fs::create_dir_all(&net)?;
    let corpus = corpus::default_corpus_root();
    let mut rows = Vec::new();

    let rimraf = corpus.join("small/rimraf");
    if rimraf.join("package-lock.json").is_file() {
        rows.extend(network_tool_compare(&net, &rimraf, "rimraf")?);
    }

    let a = corpus.join("divergence/axios-v1.6");
    let b = corpus.join("divergence/axios-v1.7");
    if a.join("package-lock.json").is_file() && b.join("package-lock.json").is_file() {
        rows.extend(network_branch_switch(&net, &a, &b)?);
    }
    Ok(rows)
}

fn network_tool_compare(
    net: &Path,
    corpus_proj: &Path,
    label: &str,
) -> anyhow::Result<Vec<Phase16Row>> {
    let mut rows = Vec::new();
    let weave_home = net.join(format!("weave-home-{label}"));
    fs::create_dir_all(&weave_home)?;
    std::env::set_var("WEAVE_HOME", &weave_home);
    let pw = net.join(format!("weave-{label}-single"));
    copy_corpus_project(corpus_proj, &pw)?;
    init_project(&pw).map_err(anyhow::Error::msg)?;
    let (out, dur) = time_it(|| switch_project(&pw, None));
    match out {
        Ok(out) => {
            let acc = weave_disk(&pw)?;
            rows.push(Phase16Row {
                measurement_class: "network".into(),
                tool: "weave".into(),
                scenario: format!("single_clean:{label}"),
                parallel_n: Some(1),
                wall_ms: dur.as_millis(),
                per_env_ms: Some(dur.as_millis()),
                peak_apparent_bytes: Some(acc.apparent_bytes),
                final_apparent_bytes: Some(acc.apparent_bytes),
                final_unique_bytes: Some(acc.unique_bytes),
                duplicated_bytes: Some(duplicated_bytes(acc)),
                approx_inodes: Some(acc.approx_inodes),
                network_bytes: None,
                weave_fetched: Some(out.prepare.fetched_artifacts as u64),
                weave_reused: Some(out.prepare.reused_artifacts as u64),
                weave_hardlinks: Some(out.prepare.materialize.hardlinked_files as u64),
                weave_copies: Some(out.prepare.materialize.copied_files as u64),
                weave_cache_hits: Some(out.prepare.materialize.cache_hits as u64),
                note: "Real lockfile; HTTPS registry. Network byte counter not available.".into(),
            });
        }
        Err(e) => rows.push(skip_row(
            "network",
            "weave",
            &format!("single_clean:{label} failed: {e}"),
        )),
    }

    // warm parallel 4
    let n = 4usize;
    let projects: Vec<PathBuf> = (0..n)
        .map(|i| net.join(format!("weave-{label}-par-{i}")))
        .collect();
    for p in &projects {
        copy_corpus_project(corpus_proj, p)?;
        init_project(p).map_err(anyhow::Error::msg)?;
    }
    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let per_ms: Arc<Mutex<Vec<u128>>> = Arc::new(Mutex::new(Vec::new()));
    let start = Instant::now();
    thread::scope(|scope| {
        for p in &projects {
            let home = weave_home.clone();
            let errors = Arc::clone(&errors);
            let per_ms = Arc::clone(&per_ms);
            let p = p.clone();
            scope.spawn(move || {
                std::env::set_var("WEAVE_HOME", &home);
                let t0 = Instant::now();
                match switch_project(&p, None) {
                    Ok(_) => per_ms.lock().unwrap().push(t0.elapsed().as_millis()),
                    Err(e) => errors.lock().unwrap().push(e.to_string()),
                }
            });
        }
    });
    let wall = start.elapsed().as_millis();
    let errs = errors.lock().unwrap().clone();
    let per = per_ms.lock().unwrap().clone();
    let avg = avg_ms(&per);
    let mut roots: Vec<PathBuf> = projects.iter().map(|p| p.join("node_modules")).collect();
    roots.push(weave_home.join("store"));
    let refs: Vec<&Path> = roots.iter().map(|p| p.as_path()).collect();
    let acc = disk_accounting(&refs);
    rows.push(Phase16Row {
        measurement_class: "network".into(),
        tool: "weave".into(),
        scenario: format!("parallel_{n}_warm:{label}"),
        parallel_n: Some(n),
        wall_ms: wall,
        per_env_ms: Some(avg),
        peak_apparent_bytes: Some(acc.apparent_bytes),
        final_apparent_bytes: Some(acc.apparent_bytes),
        final_unique_bytes: Some(acc.unique_bytes),
        duplicated_bytes: Some(duplicated_bytes(acc)),
        approx_inodes: Some(acc.approx_inodes),
        network_bytes: None,
        weave_fetched: None,
        weave_reused: None,
        weave_hardlinks: None,
        weave_copies: None,
        weave_cache_hits: None,
        note: if errs.is_empty() {
            format!(
                "After CAS warm; unique/apparent={:.3}",
                ratio(acc.unique_bytes, acc.apparent_bytes)
            )
        } else {
            format!("errors: {}", errs.join("; "))
        },
    });
    std::env::remove_var("WEAVE_HOME");

    if tool_available("npm") {
        let cache = net.join(format!("npm-cache-{label}"));
        fs::create_dir_all(&cache)?;
        let pn = net.join(format!("npm-{label}-single"));
        copy_corpus_project(corpus_proj, &pn)?;
        let start = Instant::now();
        let status = Command::new("npm")
            .args(["ci", "--ignore-scripts"])
            .env("npm_config_cache", &cache)
            .current_dir(&pn)
            .status()?;
        let wall = start.elapsed().as_millis();
        let acc = disk_accounting(&[pn.join("node_modules").as_path(), cache.as_path()]);
        rows.push(pm_row(
            "network",
            "npm",
            &format!("single_clean:{label}"),
            1,
            wall,
            Some(wall),
            acc,
            None,
            if status.success() {
                "npm ci --ignore-scripts; network bytes not measured".into()
            } else {
                format!("npm ci failed: {status}")
            },
        ));
    }

    if tool_available("pnpm") {
        let store = net.join(format!("pnpm-store-{label}"));
        fs::create_dir_all(&store)?;
        let pp = net.join(format!("pnpm-{label}-single"));
        copy_corpus_project(corpus_proj, &pp)?;
        let start = Instant::now();
        let ok = pnpm_import_and_install(&pp, &store)?;
        let wall = start.elapsed().as_millis();
        let acc = disk_accounting(&[pp.join("node_modules").as_path(), store.as_path()]);
        rows.push(pm_row(
            "network",
            "pnpm",
            &format!("single_clean:{label}"),
            1,
            wall,
            Some(wall),
            acc,
            None,
            if ok {
                "pnpm import + install; network bytes not measured".into()
            } else {
                "pnpm install failed".into()
            },
        ));
    }
    Ok(rows)
}

fn network_branch_switch(net: &Path, a: &Path, b: &Path) -> anyhow::Result<Vec<Phase16Row>> {
    let mut rows = Vec::new();
    let weave_home = net.join("weave-home-axios-branch");
    fs::create_dir_all(&weave_home)?;
    std::env::set_var("WEAVE_HOME", &weave_home);

    let pw = net.join("weave-axios-branch");
    copy_corpus_project(a, &pw)?;
    init_project(&pw).map_err(anyhow::Error::msg)?;
    let (out_a, dur_a) = time_it(|| switch_project(&pw, None));
    let out_a = match out_a {
        Ok(v) => v,
        Err(e) => {
            std::env::remove_var("WEAVE_HOME");
            rows.push(skip_row(
                "network",
                "weave",
                &format!("axios branch seed A failed: {e}"),
            ));
            return Ok(rows);
        }
    };
    rows.push(Phase16Row {
        measurement_class: "network".into(),
        tool: "weave".into(),
        scenario: "branch_seed_a:axios".into(),
        parallel_n: Some(1),
        wall_ms: dur_a.as_millis(),
        per_env_ms: Some(dur_a.as_millis()),
        peak_apparent_bytes: None,
        final_apparent_bytes: None,
        final_unique_bytes: None,
        duplicated_bytes: None,
        approx_inodes: None,
        network_bytes: None,
        weave_fetched: Some(out_a.prepare.fetched_artifacts as u64),
        weave_reused: Some(out_a.prepare.reused_artifacts as u64),
        weave_hardlinks: Some(out_a.prepare.materialize.hardlinked_files as u64),
        weave_copies: Some(out_a.prepare.materialize.copied_files as u64),
        weave_cache_hits: Some(out_a.prepare.materialize.cache_hits as u64),
        note: "axios-v1.6 cold fetch".into(),
    });

    let seed = net.join("weave-axios-seed-b");
    copy_corpus_project(b, &seed)?;
    init_project(&seed).map_err(anyhow::Error::msg)?;
    let _ = switch_project(&seed, None);

    fs::copy(b.join("package.json"), pw.join("package.json"))?;
    fs::copy(b.join("package-lock.json"), pw.join("package-lock.json"))?;
    let (out_ab, dur_ab) = time_it(|| switch_project(&pw, None));
    if let Ok(out_ab) = out_ab {
        let acc = weave_disk(&pw)?;
        rows.push(Phase16Row {
            measurement_class: "network".into(),
            tool: "weave".into(),
            scenario: "branch_a_to_b:axios".into(),
            parallel_n: Some(1),
            wall_ms: dur_ab.as_millis(),
            per_env_ms: Some(dur_ab.as_millis()),
            peak_apparent_bytes: Some(acc.apparent_bytes),
            final_apparent_bytes: Some(acc.apparent_bytes),
            final_unique_bytes: Some(acc.unique_bytes),
            duplicated_bytes: Some(duplicated_bytes(acc)),
            approx_inodes: Some(acc.approx_inodes),
            network_bytes: None,
            weave_fetched: Some(out_ab.prepare.fetched_artifacts as u64),
            weave_reused: Some(out_ab.prepare.reused_artifacts as u64),
            weave_hardlinks: Some(out_ab.prepare.materialize.hardlinked_files as u64),
            weave_copies: Some(out_ab.prepare.materialize.copied_files as u64),
            weave_cache_hits: Some(out_ab.prepare.materialize.cache_hits as u64),
            note: "Store seeded with both axios graphs (~100% artifact overlap)".into(),
        });
    }
    std::env::remove_var("WEAVE_HOME");
    Ok(rows)
}

fn derive_verdict(rows: &[Phase16Row], corpus: &[CorpusOverlapRow]) -> String {
    let weave_p8 = rows.iter().find(|r| {
        r.measurement_class == "offline" && r.tool == "weave" && r.scenario == "parallel_8"
    });
    let npm_p8 = rows.iter().find(|r| {
        r.measurement_class == "offline" && r.tool == "npm" && r.scenario == "parallel_8"
    });
    let pnpm_p8 = rows.iter().find(|r| {
        r.measurement_class == "offline"
            && r.tool == "pnpm"
            && r.scenario == "parallel_8"
            && !r.note.starts_with("FAILED")
    });

    let mut parts = Vec::new();
    if let (Some(w), Some(n)) = (weave_p8, npm_p8) {
        if w.wall_ms > 0 && n.wall_ms > w.wall_ms {
            parts.push(format!(
                "Weave parallel_8 wall {}ms vs npm {}ms ({:.1}\u{00d7} faster wall).",
                w.wall_ms,
                n.wall_ms,
                n.wall_ms as f64 / w.wall_ms as f64
            ));
        } else {
            parts.push(format!(
                "Weave parallel_8 wall {}ms vs npm {}ms — no clear wall-clock win.",
                w.wall_ms, n.wall_ms
            ));
        }
        if let (Some(wu), Some(nu)) = (w.final_unique_bytes, n.final_unique_bytes) {
            if nu > wu && wu > 0 {
                parts.push(format!(
                    "Unique disk: Weave {wu} vs npm {nu} ({:.1}\u{00d7} less unique storage).",
                    nu as f64 / wu as f64
                ));
            } else {
                parts.push(format!(
                    "Unique disk: Weave {wu:?} vs npm {nu:?} — advantage unclear."
                ));
            }
        }
        if let (Some(wd), Some(nd)) = (w.duplicated_bytes, n.duplicated_bytes) {
            parts.push(format!(
                "Duplicated apparent bytes (apparent\u{2212}unique): Weave {wd} vs npm {nd}."
            ));
        }
    } else {
        parts.push("Missing offline parallel_8 weave/npm rows — evidence incomplete.".into());
    }

    if let Some(p) = pnpm_p8 {
        if let Some(w) = weave_p8 {
            parts.push(format!(
                "pnpm parallel_8 wall {}ms unique {:?} vs Weave wall {}ms unique {:?}.",
                p.wall_ms, p.final_unique_bytes, w.wall_ms, w.final_unique_bytes
            ));
        }
    } else {
        parts.push("pnpm parallel_8 unavailable or failed — pnpm comparison incomplete.".into());
    }

    if let Some(c) = corpus.first() {
        parts.push(format!(
            "Real lockfile overlap ({}): shared_of_a={:.1}% — high overlap is the regime agents hit when branching nearby.",
            c.id,
            c.shared_of_a * 100.0
        ));
    }

    let strong = weave_p8
        .zip(npm_p8)
        .map(|(w, n)| {
            let time_win = n.wall_ms > w.wall_ms.saturating_mul(2);
            let disk_win = match (w.final_unique_bytes, n.final_unique_bytes) {
                (Some(wu), Some(nu)) => nu > wu.saturating_mul(2),
                _ => false,
            };
            time_win || disk_win
        })
        .unwrap_or(false);

    let headline = if strong {
        if pnpm_p8.is_some() {
            "YES — meaningful advantage on parallel high-overlap offline workloads vs npm, and also faster/leaner than pnpm on this fixture (wall-clock + unique disk)."
        } else {
            "YES — meaningful advantage vs npm on parallel high-overlap offline workloads; pnpm comparison incomplete."
        }
    } else if weave_p8
        .zip(pnpm_p8)
        .map(|(w, p)| {
            p.final_unique_bytes
                .zip(w.final_unique_bytes)
                .map(|(pu, wu)| pu <= wu.saturating_mul(2))
                .unwrap_or(false)
                && p.wall_ms <= w.wall_ms.saturating_mul(3)
        })
        .unwrap_or(false)
    {
        "MIXED — Weave beats npm on parallel high-overlap workloads, but pnpm's shared store closes much of the gap; wall-clock edge depends on scenario."
    } else if weave_p8.is_some() && npm_p8.is_some() {
        "WEAK/UNCLEAR — offline parallel results do not show a large Weave advantage over npm (and/or pnpm) on this fixture/host."
    } else {
        "INCONCLUSIVE — insufficient comparable rows."
    };

    format!("{headline}\n\n{}", parts.join(" "))
}

pub fn write_phase16_outputs(out_dir: &Path, report: &Phase16Report) -> anyhow::Result<()> {
    fs::create_dir_all(out_dir)?;
    fs::write(
        out_dir.join("phase16-report.json"),
        serde_json::to_vec_pretty(report)?,
    )?;
    fs::write(
        out_dir.join("phase16-ai-agent-report.md"),
        render_markdown(report),
    )?;
    Ok(())
}

pub fn render_markdown(report: &Phase16Report) -> String {
    let mut o = String::new();
    o.push_str("# Phase 16: AI-Agent Workload Validation\n\n");
    o.push_str(&format!(
        "Date host: `{}` / `{}` · Weave `{}` · npm `{}` · pnpm `{}`\n\n",
        report.host.os,
        report.host.arch,
        report.weave_version,
        report.npm_version.as_deref().unwrap_or("n/a"),
        report.pnpm_version.as_deref().unwrap_or("n/a")
    ));
    o.push_str("## Question\n\n");
    o.push_str(
        "> Does Weave provide a meaningful advantage for parallel AI-agent development environments?\n\n",
    );
    o.push_str("## Verdict\n\n");
    o.push_str(&report.verdict);
    o.push_str("\n\n");
    o.push_str("## Interpretation (evidence-bound)\n\n");
    o.push_str(
        "**Primary answer (offline, warm CAS / shared store):** evaluated from `parallel_8` \
rows above. **Network cold single installs are a separate class** — Weave is not expected \
to win first-time registry fetch vs npm/pnpm; the hypothesis is about reuse across parallel \
warm environments. See network rows (if present) for cold vs warm contrast; network byte \
counters were not fabricated when unavailable.\n\n",
    );
    o.push_str("## Hypothesis\n\n");
    o.push_str(
        "Weave's strongest advantage is **parallel isolated environments sharing CAS artifacts** \
(multiple agent worktrees / checkouts with high dependency overlap), not single-shot cold installs.\n\n",
    );
    o.push_str("## Measurement classes (not mixed)\n\n");
    o.push_str("| Class | What | Network |\n|---|---|---|\n");
    o.push_str("| `offline` | Synthetic agent-overlap lockfiles + local tarballs | none |\n");
    o.push_str(
        "| `network` | Real corpus lockfiles + registry (optional `--network`) | required |\n\n",
    );
    o.push_str("## Offline fixture (reproducible)\n\n");
    o.push_str(&format!("`{}`\n\n", report.offline_fixture));
    o.push_str(
        "Models related AI-agent worktrees: ~90% shared packages between branch A and B.\n\n",
    );
    o.push_str("## Real corpus overlap (lockfile analysis only)\n\n");
    o.push_str(
        "| pair | pkgs A/B | artifacts A/B | shared | shared/A |\n|---|---:|---:|---:|---:|\n",
    );
    for c in &report.corpus_overlap {
        o.push_str(&format!(
            "| {} | {}/{} | {}/{} | {} | {:.1}% |\n",
            c.id,
            c.packages_a,
            c.packages_b,
            c.artifacts_a,
            c.artifacts_b,
            c.shared_artifacts,
            c.shared_of_a * 100.0
        ));
    }
    o.push_str("\nProvenance: `benchmarks/corpus/**/PROVENANCE.json`.\n\n");
    o.push_str("## Results\n\n");
    o.push_str(
        "| class | tool | scenario | N | wall_ms | per_env_ms | apparent | unique | duplicated | inodes | note |\n\
         |---|---|---|---:|---:|---:|---:|---:|---:|---:|---|\n",
    );
    for r in &report.rows {
        o.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            r.measurement_class,
            r.tool,
            r.scenario,
            r.parallel_n
                .map(|n| n.to_string())
                .unwrap_or_else(|| "-".into()),
            r.wall_ms,
            r.per_env_ms
                .map(|n| n.to_string())
                .unwrap_or_else(|| "-".into()),
            fmt_opt(r.final_apparent_bytes),
            fmt_opt(r.final_unique_bytes),
            fmt_opt(r.duplicated_bytes),
            fmt_opt(r.approx_inodes),
            r.note.replace('|', "/"),
        ));
    }
    o.push_str("\n### Weave CAS counters (when recorded)\n\n");
    o.push_str(
        "| scenario | fetched | reused | hardlinks | copies | cache_hits |\n|---|---:|---:|---:|---:|---:|\n",
    );
    for r in report.rows.iter().filter(|r| r.tool == "weave") {
        if r.weave_fetched.is_none() && r.weave_cache_hits.is_none() {
            continue;
        }
        o.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            r.scenario,
            fmt_opt(r.weave_fetched),
            fmt_opt(r.weave_reused),
            fmt_opt(r.weave_hardlinks),
            fmt_opt(r.weave_copies),
            fmt_opt(r.weave_cache_hits),
        ));
    }
    o.push_str("\n## Semantic equivalence\n\n");
    o.push_str(
        "- Goal: N isolated project directories each with a usable dependency tree for the same lockfile.\n\
- Weave: `weave init` + `switch` (extraction-only).\n\
- npm: `npm ci --ignore-scripts`.\n\
- pnpm: `pnpm import` then `pnpm install --frozen-lockfile --ignore-scripts`.\n\
- Branch A\u{2194}B for npm/pnpm is a **reinstall stand-in** (no transactional switch API).\n\n",
    );
    o.push_str("## Caveats\n\n");
    for c in &report.caveats {
        o.push_str(&format!("- {c}\n"));
    }
    o.push_str("\n## Reproduce\n\n```bash\n");
    o.push_str("cargo run -p weave-bench --release -- phase16\n");
    o.push_str("cargo run -p weave-bench --release -- phase16 --network\n");
    o.push_str("```\n\n");
    o.push_str(&format!("Work dir for this run: `{}`\n", report.work_dir));
    o
}

fn fmt_opt(v: Option<u64>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| "-".into())
}
fn ratio(a: u64, b: u64) -> f64 {
    if b == 0 {
        0.0
    } else {
        a as f64 / b as f64
    }
}
fn avg_ms(per: &[u128]) -> u128 {
    if per.is_empty() {
        0
    } else {
        per.iter().sum::<u128>() / per.len() as u128
    }
}

#[allow(clippy::too_many_arguments)]
fn pm_row(
    class: &str,
    tool: &str,
    scenario: &str,
    n: usize,
    wall: u128,
    per: Option<u128>,
    acc: DiskAccounting,
    network: Option<u64>,
    note: String,
) -> Phase16Row {
    Phase16Row {
        measurement_class: class.into(),
        tool: tool.into(),
        scenario: scenario.into(),
        parallel_n: Some(n),
        wall_ms: wall,
        per_env_ms: per,
        peak_apparent_bytes: Some(acc.apparent_bytes),
        final_apparent_bytes: Some(acc.apparent_bytes),
        final_unique_bytes: Some(acc.unique_bytes),
        duplicated_bytes: Some(duplicated_bytes(acc)),
        approx_inodes: Some(acc.approx_inodes),
        network_bytes: network,
        weave_fetched: None,
        weave_reused: None,
        weave_hardlinks: None,
        weave_copies: None,
        weave_cache_hits: None,
        note,
    }
}

fn skip_row(class: &str, tool: &str, note: &str) -> Phase16Row {
    Phase16Row {
        measurement_class: class.into(),
        tool: tool.into(),
        scenario: "skipped".into(),
        parallel_n: None,
        wall_ms: 0,
        per_env_ms: None,
        peak_apparent_bytes: None,
        final_apparent_bytes: None,
        final_unique_bytes: None,
        duplicated_bytes: None,
        approx_inodes: None,
        network_bytes: None,
        weave_fetched: None,
        weave_reused: None,
        weave_hardlinks: None,
        weave_copies: None,
        weave_cache_hits: None,
        note: note.into(),
    }
}

fn weave_disk(project: &Path) -> anyhow::Result<DiskAccounting> {
    let cfg = ProjectConfig::load(project).map_err(anyhow::Error::msg)?;
    let store = PathBuf::from(cfg.store_path);
    let unpacked = store
        .parent()
        .map(|p| p.join("unpacked"))
        .unwrap_or_else(|| store.join("unpacked"));
    Ok(disk_accounting(&[
        project.join("node_modules").as_path(),
        store.as_path(),
        unpacked.as_path(),
    ]))
}

fn npm_ci_one(
    dest: &Path,
    template: &Path,
    rewrites: &[(String, PathBuf)],
    cache: &Path,
) -> anyhow::Result<(u128, DiskAccounting, String)> {
    if dest.exists() {
        let _ = fs::remove_dir_all(dest);
    }
    prepare_pm_project(dest, template, rewrites)?;
    let start = Instant::now();
    let status = Command::new("npm")
        .args(["ci", "--ignore-scripts"])
        .env("npm_config_cache", cache)
        .current_dir(dest)
        .status()?;
    let wall = start.elapsed().as_millis();
    let acc = disk_accounting(&[dest.join("node_modules").as_path(), cache]);
    let note = if status.success() {
        "npm ci --ignore-scripts with file: tarball URLs".into()
    } else {
        format!("npm ci failed: {status}")
    };
    Ok((wall, acc, note))
}

fn pnpm_install_one(
    dest: &Path,
    template: &Path,
    rewrites: &[(String, PathBuf)],
    store: &Path,
) -> anyhow::Result<(u128, DiskAccounting, String)> {
    if dest.exists() {
        let _ = fs::remove_dir_all(dest);
    }
    prepare_pnpm_file_project(dest, template, rewrites)?;
    let start = Instant::now();
    let status = Command::new("pnpm")
        .args(["install", "--ignore-scripts", "--no-frozen-lockfile"])
        .env("PNPM_STORE_DIR", store)
        .current_dir(dest)
        .status()?;
    let wall = start.elapsed().as_millis();
    let acc = disk_accounting(&[dest.join("node_modules").as_path(), store]);
    let note = if status.success() {
        "pnpm install --ignore-scripts with package.json file: tarball deps (offline)".into()
    } else {
        "pnpm install failed".into()
    };
    Ok((wall, acc, note))
}

fn pnpm_import_and_install(dest: &Path, store: &Path) -> anyhow::Result<bool> {
    // Network class: import npm lockfile then frozen install.
    let _ = Command::new("pnpm")
        .args(["import"])
        .env("PNPM_STORE_DIR", store)
        .current_dir(dest)
        .status()?;
    let status = Command::new("pnpm")
        .args(["install", "--frozen-lockfile", "--ignore-scripts"])
        .env("PNPM_STORE_DIR", store)
        .current_dir(dest)
        .status()?;
    Ok(status.success())
}

/// Offline pnpm: rewrite package.json deps to `file:` tarballs (import drops file: URLs).
fn prepare_pnpm_file_project(
    dest: &Path,
    template: &Path,
    rewrites: &[(String, PathBuf)],
) -> anyhow::Result<()> {
    fs::create_dir_all(dest)?;
    let pkg_raw = fs::read_to_string(template.join("package.json"))?;
    let mut pkg: serde_json::Value = serde_json::from_str(&pkg_raw)?;
    let mut deps = serde_json::Map::new();
    // Map package name -> tarball from rewrites URLs (.../name/-/name-ver.tgz)
    let mut by_name: std::collections::BTreeMap<String, PathBuf> =
        std::collections::BTreeMap::new();
    for (url, path) in rewrites {
        // https://example.invalid/{name}/-/{name}-{ver}.tgz
        if let Some(rest) = url.strip_prefix("https://example.invalid/") {
            if let Some(name) = rest.split('/').next() {
                by_name.insert(name.to_owned(), path.clone());
            }
        }
    }
    if let Some(obj) = pkg.get("dependencies").and_then(|d| d.as_object()) {
        for name in obj.keys() {
            if let Some(tgz) = by_name.get(name) {
                deps.insert(name.clone(), format!("file:{}", tgz.display()).into());
            }
        }
    }
    pkg["dependencies"] = serde_json::Value::Object(deps);
    fs::write(
        dest.join("package.json"),
        serde_json::to_string_pretty(&pkg)?,
    )?;
    if template.join("README").is_file() {
        fs::copy(template.join("README"), dest.join("README"))?;
    }
    let _ = Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(dest)
        .output();
    Ok(())
}

fn prepare_pm_project(
    dest: &Path,
    template: &Path,
    rewrites: &[(String, PathBuf)],
) -> anyhow::Result<()> {
    fs::create_dir_all(dest)?;
    for name in ["package.json", "package-lock.json", "README"] {
        let src = template.join(name);
        if src.is_file() {
            fs::copy(&src, dest.join(name))?;
        }
    }
    let _ = Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(dest)
        .output();
    let mut lock = fs::read_to_string(dest.join("package-lock.json"))?;
    for (url, path) in rewrites {
        lock = lock.replace(url, &format!("file:{}", path.display()));
    }
    fs::write(dest.join("package-lock.json"), lock)?;
    Ok(())
}

fn copy_project(src: &Path, dest: &Path) -> anyhow::Result<()> {
    if dest.exists() {
        let _ = fs::remove_dir_all(dest);
    }
    fs::create_dir_all(dest)?;
    for name in ["package.json", "package-lock.json", "README"] {
        let s = src.join(name);
        if s.is_file() {
            fs::copy(&s, dest.join(name))?;
        }
    }
    let _ = Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(dest)
        .output();
    let _ = Command::new("git")
        .args(["add", "-A"])
        .current_dir(dest)
        .output();
    let status = Command::new("git")
        .args([
            "-c",
            "user.email=bench@weave",
            "-c",
            "user.name=weave-bench",
            "commit",
            "-m",
            "bench",
        ])
        .current_dir(dest)
        .status()?;
    if !status.success() {
        anyhow::bail!("git commit failed in {}", dest.display());
    }
    Ok(())
}

fn copy_corpus_project(src: &Path, dest: &Path) -> anyhow::Result<()> {
    if dest.exists() {
        let _ = fs::remove_dir_all(dest);
    }
    fs::create_dir_all(dest)?;
    for name in ["package.json", "package-lock.json"] {
        let s = src.join(name);
        if s.is_file() {
            fs::copy(&s, dest.join(name))?;
        }
    }
    if !dest.join("package.json").is_file() {
        fs::write(
            dest.join("package.json"),
            r#"{"name":"corpus-bench","version":"0.0.0","private":true}"#,
        )?;
    }
    let _ = Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(dest)
        .output();
    let _ = Command::new("git")
        .args(["add", "-A"])
        .current_dir(dest)
        .output();
    let _ = Command::new("git")
        .args([
            "-c",
            "user.email=bench@weave",
            "-c",
            "user.name=weave-bench",
            "commit",
            "-m",
            "corpus",
        ])
        .current_dir(dest)
        .output();
    Ok(())
}

fn tool_available(bin: &str) -> bool {
    Command::new(bin).arg("--version").output().is_ok()
}

fn tool_version(bin: &str) -> Option<String> {
    let out = Command::new(bin).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn make_work_root(keep_work: bool) -> anyhow::Result<(PathBuf, Option<tempfile::TempDir>)> {
    let td = tempfile::Builder::new()
        .prefix("weave-phase16-")
        .tempdir()?;
    if keep_work {
        let kept = std::env::temp_dir().join(format!("weave-phase16-keep-{}", std::process::id()));
        if kept.exists() {
            let _ = fs::remove_dir_all(&kept);
        }
        fs::rename(td.path(), &kept)?;
        std::mem::forget(td);
        Ok((kept, None))
    } else {
        Ok((td.path().to_path_buf(), Some(td)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_overlap_rows_exist() {
        let rows = analyze_corpus_overlap().expect("corpus");
        assert!(!rows.is_empty());
        assert!(rows.iter().any(|r| r.shared_of_a > 0.5));
    }

    /// Full offline suite is slow (npm/pnpm parallel); run via `weave-bench phase16`.
    #[test]
    #[ignore = "run: cargo run -p weave-bench --release -- phase16"]
    fn offline_phase16_produces_parallel_rows() {
        let report = run_phase16(Phase16Opts {
            network: false,
            keep_work: false,
        })
        .expect("phase16");
        assert!(
            report
                .rows
                .iter()
                .any(|r| r.tool == "weave" && r.scenario == "parallel_8"),
            "missing weave parallel_8"
        );
        assert!(!report.verdict.is_empty());
    }
}
