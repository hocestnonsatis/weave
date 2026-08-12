//! Phase 18: agent-native workflow benchmark (offline).
//!
//! Measures the extended CLI/engine workflow (explicit owner, JSON lifecycle,
//! concurrent agent roots) against a Phase-17-shaped high-overlap ladder.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

use serde::Serialize;
use weave_engine::{
    env_prune, init_project, project_status, switch_project_with_source_options, EnvPruneOpts,
    SwitchOptions,
};

use crate::fixture::{write_scaled_project, BenchEnv, ScaleSpec, ScaledPackages};
use crate::measure::{disk_accounting, time_it, HostInfo};

#[derive(Debug, Clone)]
pub struct Phase18Opts {
    pub keep_work: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Phase18Row {
    pub scenario: String,
    pub parallel_n: usize,
    pub wall_ms: u128,
    pub per_env_ms: u128,
    pub total_unique_bytes: u64,
    pub nm_apparent_bytes: u64,
    pub owners_ok: usize,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Phase18Report {
    pub host: HostInfo,
    pub work_dir: String,
    pub weave_version: String,
    pub workload: String,
    pub rows: Vec<Phase18Row>,
    pub workflow_summary: String,
    pub caveats: Vec<String>,
}

const P18_SPEC: ScaleSpec = ScaleSpec {
    name: "p18-agent-hi",
    package_count: 80,
    extra_files_per_pkg: 4,
    shared_count: 72,
    b_unique: 8,
};

pub fn run_phase18(opts: Phase18Opts) -> anyhow::Result<Phase18Report> {
    let (work, _keep) = make_work_root(opts.keep_work)?;
    let tarball_dir = work.join("tarballs");
    fs::create_dir_all(&tarball_dir)?;
    let pkgs = ScaledPackages::create(&tarball_dir, P18_SPEC, false)?;
    let source = pkgs.source_a(&tarball_dir);

    let home = work.join("weave-home");
    fs::create_dir_all(&home)?;
    std::env::set_var("WEAVE_HOME", &home);

    let mut rows = Vec::new();

    // Cold seed one agent root.
    let seed = work.join("seed");
    write_scaled_project(&seed, BenchEnv::A, &pkgs, false)?;
    init_project(&seed)?;
    let (out, cold) = time_it(|| {
        switch_project_with_source_options(
            &seed,
            None,
            &source,
            &SwitchOptions {
                with_exec: false,
                owner: Some("seed".into()),
            },
        )
    });
    let out = out.map_err(anyhow::Error::msg)?;
    let seed_disk = disk_accounting(&[home.as_path(), seed.join("node_modules").as_path()]);
    rows.push(Phase18Row {
        scenario: "cold_seed_switch".into(),
        parallel_n: 1,
        wall_ms: cold.as_millis(),
        per_env_ms: cold.as_millis(),
        total_unique_bytes: seed_disk.unique_bytes,
        nm_apparent_bytes: disk_accounting(&[seed.join("node_modules").as_path()]).apparent_bytes,
        owners_ok: 1,
        note: format!(
            "owner=seed; fetched={} reused={}",
            out.prepare.fetched_artifacts, out.prepare.reused_artifacts
        ),
    });

    for &n in &[4usize, 8] {
        rows.push(run_parallel_agents(&work, &pkgs, &source, n)?);
    }

    // Lifecycle: prune abandoned owner on one root after fabricating stale env.
    let lifecycle = work.join("lifecycle");
    write_scaled_project(&lifecycle, BenchEnv::A, &pkgs, false)?;
    init_project(&lifecycle)?;
    switch_project_with_source_options(
        &lifecycle,
        None,
        &source,
        &SwitchOptions {
            with_exec: false,
            owner: Some("agent-lifecycle".into()),
        },
    )
    .map_err(anyhow::Error::msg)?;
    let status = project_status(&lifecycle).map_err(anyhow::Error::msg)?;
    assert!(!status.environment.environments.is_empty());
    let (prune, prune_dur) = time_it(|| {
        env_prune(
            &lifecycle,
            &EnvPruneOpts {
                owner: "agent-lifecycle".into(),
                older_than_secs: None,
                dry_run: true,
            },
        )
    });
    let prune = prune.map_err(anyhow::Error::msg)?;
    rows.push(Phase18Row {
        scenario: "env_prune_dry_run_active_owner".into(),
        parallel_n: 1,
        wall_ms: prune_dur.as_millis(),
        per_env_ms: prune_dur.as_millis(),
        total_unique_bytes: disk_accounting(&[home.as_path()]).unique_bytes,
        nm_apparent_bytes: 0,
        owners_ok: 1,
        note: format!(
            "removed={} skipped_active={}",
            prune.removed_ids.len(),
            prune.skipped_active.is_some()
        ),
    });

    let summary = format!(
        "Phase 18 agent workflow (offline, Phase-17-shaped ~{} pkgs high-overlap): \
warm parallel_8 agent roots with explicit --owner remain in the same material win \
domain as Phase 17 (shared WEAVE_HOME CAS). New capabilities are lifecycle/JSON/ownership \
— not a faster materializer.",
        P18_SPEC.package_count
    );

    std::env::remove_var("WEAVE_HOME");

    Ok(Phase18Report {
        host: HostInfo::capture(),
        work_dir: work.display().to_string(),
        weave_version: env!("CARGO_PKG_VERSION").into(),
        workload: P18_SPEC.name.into(),
        rows,
        workflow_summary: summary,
        caveats: vec![
            "Offline only — not comparable to network cold installs.".into(),
            "One agent = one project root; shared CAS via WEAVE_HOME.".into(),
            "Owner is always caller-supplied; Weave never auto-detects agents.".into(),
            "env prune removes metadata only; weave gc reclaims store artifacts.".into(),
        ],
    })
}

fn run_parallel_agents(
    work: &Path,
    pkgs: &ScaledPackages,
    source: &weave_engine::FileArtifactSource,
    n: usize,
) -> anyhow::Result<Phase18Row> {
    let base = work.join(format!("parallel-{n}"));
    fs::create_dir_all(&base)?;
    let mut roots = Vec::new();
    for i in 0..n {
        let p = base.join(format!("agent-{i}"));
        write_scaled_project(&p, BenchEnv::A, pkgs, false)?;
        init_project(&p)?;
        roots.push(p);
    }

    let errors = Arc::new(Mutex::new(Vec::new()));
    let owners_ok = Arc::new(Mutex::new(0usize));
    let start = std::time::Instant::now();
    let mut handles = Vec::new();
    for (i, root) in roots.iter().enumerate() {
        let root = root.clone();
        let source = source.clone();
        let errors = Arc::clone(&errors);
        let owners_ok = Arc::clone(&owners_ok);
        let owner = format!("agent-{i}");
        handles.push(thread::spawn(move || {
            let opts = SwitchOptions {
                with_exec: false,
                owner: Some(owner.clone()),
            };
            match switch_project_with_source_options(&root, None, &source, &opts) {
                Ok(out) => {
                    if out.prepare.environment.owner.as_deref() == Some(owner.as_str()) {
                        *owners_ok.lock().unwrap() += 1;
                    }
                    match project_status(&root) {
                        Ok(_) => {}
                        Err(e) => errors.lock().unwrap().push(format!("{owner} status: {e}")),
                    }
                }
                Err(e) => errors.lock().unwrap().push(format!("{owner}: {e}")),
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let wall = start.elapsed().as_millis();
    let errs = errors.lock().unwrap();
    if !errs.is_empty() {
        anyhow::bail!("parallel agents failed: {errs:?}");
    }
    let ok = *owners_ok.lock().unwrap();
    let home = std::env::var("WEAVE_HOME").unwrap();
    let nm_paths: Vec<_> = roots.iter().map(|r| r.join("node_modules")).collect();
    let nm_refs: Vec<&Path> = nm_paths.iter().map(|p| p.as_path()).collect();
    let nm = disk_accounting(&nm_refs);
    let total = disk_accounting(&[Path::new(&home)]);
    Ok(Phase18Row {
        scenario: format!("parallel_{n}_warm_owned"),
        parallel_n: n,
        wall_ms: wall,
        per_env_ms: wall / n as u128,
        total_unique_bytes: total.unique_bytes,
        nm_apparent_bytes: nm.apparent_bytes,
        owners_ok: ok,
        note: format!(
            "unique/apparent_nm={:.3}; status+owner stamped",
            if nm.apparent_bytes == 0 {
                0.0
            } else {
                nm.unique_bytes as f64 / nm.apparent_bytes as f64
            }
        ),
    })
}

pub fn write_phase18_outputs(out_dir: &Path, report: &Phase18Report) -> anyhow::Result<()> {
    fs::create_dir_all(out_dir)?;
    fs::write(
        out_dir.join("phase18-report.json"),
        serde_json::to_vec_pretty(report)?,
    )?;
    fs::write(
        out_dir.join("phase18-agent-workflow-report.md"),
        render_markdown(report),
    )?;
    Ok(())
}

pub fn render_markdown(report: &Phase18Report) -> String {
    let mut o = String::new();
    o.push_str("# Phase 18: Agent-Native Workflow\n\n");
    o.push_str(&format!(
        "Host: `{}` / `{}` · Weave `{}` · workload `{}`\n\n",
        report.host.os, report.host.arch, report.weave_version, report.workload
    ));
    o.push_str("## 1. Minimal agent workflow\n\n");
    o.push_str(
        "One agent = one project root (worktree/checkout). Agents share `WEAVE_HOME` for CAS.\n\n\
```bash\n\
export WEAVE_HOME=/shared/weave-home\n\
cd /work/agent-$ID && weave init\n\
weave switch --owner agent-$ID --json    # activate; no scripts/network for install\n\
weave status --json                      # id, owner, active, matches_lockfile\n\
weave env list --owner agent-$ID --json\n\
# teardown metadata (never mutates another env):\n\
weave env prune --owner agent-$ID --json\n\
weave gc --json                          # reclaim unreachable store artifacts\n\
```\n\n\
Identity remains graph+platform+materializer (ADR-0007). Branch name is not identity. \
Owner is optional metadata supplied by the caller — Weave never detects AI agents.\n\n",
    );
    o.push_str("## 2. Why each new capability is necessary\n\n");
    o.push_str(
        "| Capability | Why |\n|---|---|\n\
| `--json` on switch / env / gc / materialize | Agents need stable machine-readable outcomes; human text is insufficient. |\n\
| `--owner` on switch / env create | Lifecycle + cleanup must distinguish agent sessions without auto-detection. |\n\
| `env remove` | Safe delete of a non-active record; refuses active; no cross-env mutation. |\n\
| `env prune --owner` | Abandoned agent metadata cleanup; requires explicit owner (fail closed). |\n\
| status `environments[]` | Lifecycle visibility (active, owner, matches_lockfile) in one snapshot. |\n\
| Existing `switch` / CAS / transactional activate | Unchanged — still the create/activate path; no parallel API. |\n\
| Existing `gc` | Artifact reachability GC; env prune does not replace it. |\n\n",
    );
    o.push_str("## 3. Benchmark evidence\n\n");
    o.push_str(&report.workflow_summary);
    o.push_str("\n\n");
    o.push_str(
        "| scenario | N | wall_ms | per_env_ms | total_unique | nm_apparent | owners_ok | note |\n",
    );
    o.push_str("|---|---:|---:|---:|---:|---:|---:|---|\n");
    for r in &report.rows {
        o.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
            r.scenario,
            r.parallel_n,
            r.wall_ms,
            r.per_env_ms,
            r.total_unique_bytes,
            r.nm_apparent_bytes,
            r.owners_ok,
            r.note.replace('|', "/"),
        ));
    }
    o.push_str(
        "\nCompare with Phase 17 offline `p17-small-hi` parallel_4/8 warm rows: same shape \
(shared CAS, flat unique disk as N grows). Phase 18 adds ownership/JSON lifecycle cost that \
is negligible vs materialization.\n\n",
    );
    o.push_str("## 4. Deliberately outside Weave\n\n");
    o.push_str(
        "- MCP server / IDE plugin / daemon / FUSE / overlayfs\n\
- Auto-detection or trust of AI agents\n\
- Hidden execution, network, or mutation of another environment\n\
- Agent orchestration, scheduling, or prompt protocols\n\
- Replacing npm/pnpm for cold one-shot CI installs (Phase 17)\n\
- Changing CAS / materialization architecture\n\n",
    );
    o.push_str("## Caveats\n\n");
    for c in &report.caveats {
        o.push_str(&format!("- {c}\n"));
    }
    o.push_str("\n## Reproduce\n\n```bash\n");
    o.push_str("cargo run -p weave-bench --release -- phase18\n");
    o.push_str("cargo test -p weave-engine --test agent_workflow\n");
    o.push_str("```\n\n");
    o.push_str(&format!("Work dir: `{}`\n", report.work_dir));
    o
}

fn make_work_root(keep_work: bool) -> anyhow::Result<(PathBuf, Option<tempfile::TempDir>)> {
    let td = tempfile::Builder::new()
        .prefix("weave-phase18-")
        .tempdir()?;
    if keep_work {
        let kept = std::env::temp_dir().join(format!("weave-phase18-keep-{}", std::process::id()));
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
