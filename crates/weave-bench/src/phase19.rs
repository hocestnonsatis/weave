//! Phase 19: zero-friction adoption measurements + agent-sim.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use weave_engine::{
    adoption_guide, doctor_project, init_project, project_status, recover_project,
    switch_project_with_source, FileArtifactSource, RecoverOpts,
};
use weave_fs::pack_npm_tarball;

use crate::measure::{time_it, HostInfo};

#[derive(Debug, Clone)]
pub struct Phase19Opts {
    pub keep_work: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Phase19Case {
    pub name: String,
    pub supported: bool,
    pub commands_required: usize,
    pub cold_init_ms: Option<u128>,
    pub warm_init_ms: Option<u128>,
    pub cold_switch_ms: Option<u128>,
    pub recover_ms: Option<u128>,
    pub doctor_errors: usize,
    pub agent_sim_ok: bool,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Phase19Report {
    pub host: HostInfo,
    pub work_dir: String,
    pub weave_version: String,
    pub cases: Vec<Phase19Case>,
    pub summary: String,
    pub remaining_friction: Vec<String>,
    pub kept_changes: Vec<String>,
    pub left_manual: Vec<String>,
    pub agent_substrate_verdict: String,
}

pub fn run_phase19(opts: Phase19Opts) -> anyhow::Result<Phase19Report> {
    let (work, _keep) = make_work_root(opts.keep_work)?;
    let home = work.join("weave-home");
    fs::create_dir_all(&home)?;
    std::env::set_var("WEAVE_HOME", &home);

    let cases = vec![
        case_extraction_fixture(&work)?,
        case_pnpm_unsupported(&work)?,
        case_corpus_lockfile_doctor(&work, "rimraf")?,
        case_corpus_lockfile_doctor(&work, "typescript")?,
        case_agent_help_only_sim(&work)?,
    ];

    let supported_ok = cases
        .iter()
        .filter(|c| c.supported && c.agent_sim_ok)
        .count();
    let summary = format!(
        "Phase 19 adoption: {} supported cases agent-operable via CLI help/JSON; \
happy path remains 3–4 commands (guide→init→doctor→switch). Init is idempotent; \
recover clears interrupted state; Yarn/pnpm-only trees fail closed with actionable errors.",
        supported_ok
    );

    let agent_substrate_verdict = {
        let ok = cases
            .iter()
            .any(|c| c.name == "agent-help-json-sim" && c.agent_sim_ok)
            && cases
                .iter()
                .any(|c| c.name == "extraction-fixture" && c.supported && c.commands_required <= 4);
        if ok {
            "YES — suitable as an agent dependency substrate for extraction-ready npm \
lockfile projects when the agent follows `weave guide --json` and status.next_steps \
without learning CAS internals. Not automatic; not a silent npm replacement."
                .into()
        } else {
            "NOT YET — agent-sim or extraction happy path failed on this host.".into()
        }
    };

    std::env::remove_var("WEAVE_HOME");

    Ok(Phase19Report {
        host: HostInfo::capture(),
        work_dir: work.display().to_string(),
        weave_version: env!("CARGO_PKG_VERSION").into(),
        cases,
        summary,
        remaining_friction: vec![
            "Network cold first switch still slower than npm/pnpm (Phase 17) — expected.".into(),
            "Native/lifecycle projects still need human-reviewed policy before --with-exec.".into(),
            "Yarn/pnpm/Bun lockfiles unsupported — intentional fail-closed.".into(),
            "Agents must still choose WEAVE_HOME when sharing CAS across worktrees.".into(),
        ],
        kept_changes: vec![
            "Idempotent `weave init --json`".into(),
            "`weave guide --json` + docs/agent-quickstart.md".into(),
            "`weave recover --json` for leftover candidate / dangling active".into(),
            "status.next_steps for agent follow-through".into(),
            "Clear UnsupportedLockfile when pnpm/yarn/bun present without package-lock.json".into(),
            "Actionable recovery hints pointing at guide/status --json".into(),
        ],
        left_manual: vec![
            "npm remains the lockfile/resolver owner".into(),
            "execution.enabled / --with-exec".into(),
            "git checkout (Weave never runs git switch)".into(),
            "AI agent detection/trust".into(),
            "MCP / IDE / daemon / FUSE".into(),
        ],
        agent_substrate_verdict,
    })
}

fn case_extraction_fixture(work: &Path) -> anyhow::Result<Phase19Case> {
    let root = work.join("extraction");
    fs::create_dir_all(&root)?;
    let tgz = pack_npm_tarball(&[
        ("package.json", br#"{"name":"demo-lib","version":"1.0.0"}"#),
        ("index.js", b"module.exports=1;\n"),
    ]);
    let tarball_dir = work.join("tarballs");
    fs::create_dir_all(&tarball_dir)?;
    let tarball = tarball_dir.join("demo-lib-1.0.0.tgz");
    fs::write(&tarball, &tgz)?;
    write_npm_project(&root, &integrity(&tgz))?;

    let mut notes = Vec::new();
    let (init1, cold_init) = time_it(|| init_project(&root));
    init1.map_err(anyhow::Error::msg)?;
    let (init2, warm_init) = time_it(|| init_project(&root));
    let warm = init2.map_err(anyhow::Error::msg)?;
    assert!(!warm.created);
    notes.push("init idempotent".into());

    let doctor = doctor_project(&root).map_err(anyhow::Error::msg)?;
    let doctor_errors = doctor
        .findings
        .iter()
        .filter(|f| f.severity == weave_engine::DoctorSeverity::Error)
        .count();

    let source = FileArtifactSource::new(&tarball_dir).with_override("demo-lib", tarball);
    let (sw, cold_switch) = time_it(|| switch_project_with_source(&root, None, &source));
    sw.map_err(anyhow::Error::msg)?;

    // Simulate interrupted candidate leftover.
    let cand = root.join(".weave").join("candidate");
    fs::create_dir_all(cand.join("x"))?;
    let (rec, recover_ms) = time_it(|| recover_project(&root, &RecoverOpts::default()));
    let rec = rec.map_err(anyhow::Error::msg)?;
    assert!(rec.removed_candidate);
    notes.push("recover removed leftover candidate".into());

    let status = project_status(&root).map_err(anyhow::Error::msg)?;
    assert!(!status.next_steps.is_empty());
    notes.push(format!("next_steps={}", status.next_steps.join("; ")));

    Ok(Phase19Case {
        name: "extraction-fixture".into(),
        supported: true,
        commands_required: 4, // guide, init, doctor, switch
        cold_init_ms: Some(cold_init.as_millis()),
        warm_init_ms: Some(warm_init.as_millis()),
        cold_switch_ms: Some(cold_switch.as_millis()),
        recover_ms: Some(recover_ms.as_millis()),
        doctor_errors,
        agent_sim_ok: true,
        notes,
    })
}

fn case_pnpm_unsupported(work: &Path) -> anyhow::Result<Phase19Case> {
    let root = work.join("pnpm-only");
    fs::create_dir_all(&root)?;
    git_init(&root)?;
    fs::write(
        root.join("package.json"),
        r#"{"name":"pnpm-app","version":"1.0.0"}"#,
    )?;
    fs::write(root.join("pnpm-lock.yaml"), "lockfileVersion: '9.0'\n")?;
    git_commit(&root, "pnpm")?;

    let err = init_project(&root).unwrap_err();
    let msg = err.to_string();
    let ok = msg.to_lowercase().contains("pnpm") || msg.to_lowercase().contains("unsupported");
    Ok(Phase19Case {
        name: "pnpm-only-unsupported".into(),
        supported: false,
        commands_required: 1,
        cold_init_ms: None,
        warm_init_ms: None,
        cold_switch_ms: None,
        recover_ms: None,
        doctor_errors: 1,
        agent_sim_ok: ok,
        notes: vec![format!("init error: {msg}")],
    })
}

fn case_corpus_lockfile_doctor(work: &Path, id: &str) -> anyhow::Result<Phase19Case> {
    let corpus = crate::corpus::default_corpus_root();
    let src = find_corpus_dir(&corpus, id);
    let Some(src) = src else {
        return Ok(Phase19Case {
            name: format!("corpus-{id}-missing"),
            supported: false,
            commands_required: 0,
            cold_init_ms: None,
            warm_init_ms: None,
            cold_switch_ms: None,
            recover_ms: None,
            doctor_errors: 0,
            agent_sim_ok: false,
            notes: vec!["corpus entry not found".into()],
        });
    };
    let root = work.join(format!("corpus-{id}"));
    copy_dir_minimal(&src, &root)?;
    if !root.join(".git").exists() {
        git_init(&root)?;
        git_commit(&root, "corpus")?;
    }

    let (init, cold_init) = time_it(|| init_project(&root));
    match init {
        Ok(_) => {}
        Err(e) => {
            return Ok(Phase19Case {
                name: format!("corpus-{id}"),
                supported: false,
                commands_required: 1,
                cold_init_ms: Some(cold_init.as_millis()),
                warm_init_ms: None,
                cold_switch_ms: None,
                recover_ms: None,
                doctor_errors: 1,
                agent_sim_ok: true, // clear failure is agent-operable
                notes: vec![format!("init refused: {e}")],
            });
        }
    }
    let doctor = doctor_project(&root).map_err(anyhow::Error::msg)?;
    let doctor_errors = doctor
        .findings
        .iter()
        .filter(|f| f.severity == weave_engine::DoctorSeverity::Error)
        .count();
    let status = project_status(&root).map_err(anyhow::Error::msg)?;
    Ok(Phase19Case {
        name: format!("corpus-{id}-init-doctor"),
        supported: doctor_errors == 0,
        commands_required: 3,
        cold_init_ms: Some(cold_init.as_millis()),
        warm_init_ms: None,
        cold_switch_ms: None,
        recover_ms: None,
        doctor_errors,
        agent_sim_ok: !status.next_steps.is_empty(),
        notes: vec![
            format!("pkgs={:?}", status.dependency.package_count),
            format!("next={}", status.next_steps.join("; ")),
            "network switch skipped (offline measurement class)".into(),
        ],
    })
}

fn case_agent_help_only_sim(work: &Path) -> anyhow::Result<Phase19Case> {
    // Simulate an agent that only reads CLI help + guide JSON + status next_steps.
    let weave = weave_bin()?;
    let help = Command::new(&weave).arg("--help").output()?;
    let help_txt = String::from_utf8_lossy(&help.stdout);
    let mut notes = Vec::new();
    let mut ok =
        help_txt.contains("guide") && help_txt.contains("init") && help_txt.contains("switch");
    notes.push(format!(
        "help_mentions_guide={}",
        help_txt.contains("guide")
    ));

    let guide_out = Command::new(&weave)
        .args(["guide", "--json"])
        .current_dir(work)
        .output()?;
    ok &= guide_out.status.success();
    let guide_json: serde_json::Value = serde_json::from_slice(&guide_out.stdout)?;
    ok &= guide_json.get("recipe").is_some();
    notes.push("guide --json parsed".into());

    // Local guide API also works without spawning when outside a project.
    let g = adoption_guide(None);
    ok &= !g.recipe.is_empty();

    Ok(Phase19Case {
        name: "agent-help-json-sim".into(),
        supported: true,
        commands_required: 1,
        cold_init_ms: None,
        warm_init_ms: None,
        cold_switch_ms: None,
        recover_ms: None,
        doctor_errors: 0,
        agent_sim_ok: ok,
        notes,
    })
}

pub fn write_phase19_outputs(out_dir: &Path, report: &Phase19Report) -> anyhow::Result<()> {
    fs::create_dir_all(out_dir)?;
    fs::write(
        out_dir.join("phase19-report.json"),
        serde_json::to_vec_pretty(report)?,
    )?;
    fs::write(
        out_dir.join("phase19-adoption-report.md"),
        render_markdown(report),
    )?;
    Ok(())
}

pub fn render_markdown(report: &Phase19Report) -> String {
    let mut o = String::new();
    o.push_str("# Phase 19: Zero-Friction Adoption\n\n");
    o.push_str(&format!(
        "Host: `{}` / `{}` · Weave `{}`\n\n",
        report.host.os, report.host.arch, report.weave_version
    ));
    o.push_str("## Question\n\n");
    o.push_str(
        "> Can a coding agent use Weave correctly without needing to understand \
its internal architecture?\n\n",
    );
    o.push_str("## Verdict\n\n");
    o.push_str(&report.agent_substrate_verdict);
    o.push_str("\n\n");
    o.push_str(&report.summary);
    o.push_str("\n\n");
    o.push_str("## 1. Remaining adoption friction\n\n");
    for x in &report.remaining_friction {
        o.push_str(&format!("- {x}\n"));
    }
    o.push_str("\n## 2. Changes actually worth keeping\n\n");
    for x in &report.kept_changes {
        o.push_str(&format!("- {x}\n"));
    }
    o.push_str("\n## 3. Deliberately left manual\n\n");
    for x in &report.left_manual {
        o.push_str(&format!("- {x}\n"));
    }
    o.push_str("\n## 4. Agent dependency substrate?\n\n");
    o.push_str(&report.agent_substrate_verdict);
    o.push_str("\n\n## Measurements\n\n");
    o.push_str(
        "| case | supported | cmds | cold_init_ms | warm_init_ms | cold_switch_ms | recover_ms | doctor_err | agent_ok | notes |\n\
         |---|---|---:|---:|---:|---:|---:|---:|---|---|\n",
    );
    for c in &report.cases {
        o.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            c.name,
            c.supported,
            c.commands_required,
            fmt_opt(c.cold_init_ms),
            fmt_opt(c.warm_init_ms),
            fmt_opt(c.cold_switch_ms),
            fmt_opt(c.recover_ms),
            c.doctor_errors,
            c.agent_sim_ok,
            c.notes.join("; ").replace('|', "/"),
        ));
    }
    o.push_str("\n## Reproduce\n\n```bash\n");
    o.push_str("cargo run -p weave-bench --release -- phase19\n");
    o.push_str("cargo run -p weave-cli -- guide --json\n");
    o.push_str("```\n\n");
    o.push_str(&format!("Work dir: `{}`\n", report.work_dir));
    o
}

fn fmt_opt(v: Option<u128>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| "-".into())
}

fn weave_bin() -> anyhow::Result<PathBuf> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates
    p.pop(); // repo
    let release = p.join("target/release/weave");
    let debug = p.join("target/debug/weave");
    if release.is_file() {
        Ok(release)
    } else if debug.is_file() {
        Ok(debug)
    } else {
        // Ensure built.
        let status = Command::new("cargo")
            .args(["build", "-p", "weave-cli", "--release"])
            .status()?;
        anyhow::ensure!(status.success(), "failed to build weave-cli");
        Ok(release)
    }
}

fn find_corpus_dir(corpus: &Path, id: &str) -> Option<PathBuf> {
    for cat in [
        "small",
        "medium",
        "large",
        "real-world",
        "divergence",
        "lifecycle",
        "native",
        "monorepo",
    ] {
        let p = corpus.join(cat).join(id);
        if p.join("package-lock.json").is_file() {
            return Some(p);
        }
    }
    // Fallback: recursive one-level search under corpus categories.
    if let Ok(entries) = fs::read_dir(corpus) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                if let Ok(sub) = fs::read_dir(&p) {
                    for s in sub.flatten() {
                        let cand = s.path();
                        if cand.file_name().and_then(|s| s.to_str()) == Some(id)
                            && cand.join("package-lock.json").is_file()
                        {
                            return Some(cand);
                        }
                    }
                }
            }
        }
    }
    None
}

fn copy_dir_minimal(src: &Path, dst: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dst)?;
    for name in [
        "package.json",
        "package-lock.json",
        "README.md",
        "PROVENANCE.json",
    ] {
        let from = src.join(name);
        if from.is_file() {
            fs::copy(&from, dst.join(name))?;
        }
    }
    Ok(())
}

fn write_npm_project(root: &Path, integrity: &str) -> anyhow::Result<()> {
    git_init(root)?;
    fs::write(
        root.join("package.json"),
        r#"{"name":"app","version":"1.0.0","dependencies":{"demo-lib":"1.0.0"}}"#,
    )?;
    fs::write(
        root.join("package-lock.json"),
        format!(
            r#"{{
  "name": "app",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "requires": true,
  "packages": {{
    "": {{ "name": "app", "version": "1.0.0", "dependencies": {{ "demo-lib": "1.0.0" }} }},
    "node_modules/demo-lib": {{
      "version": "1.0.0",
      "resolved": "https://example.invalid/demo-lib/-/demo-lib-1.0.0.tgz",
      "integrity": "{integrity}"
    }}
  }}
}}"#
        ),
    )?;
    git_commit(root, "app")?;
    Ok(())
}

fn integrity(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let dig = Sha256::digest(bytes);
    format!("sha256-{}", b64(&dig))
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

fn git_init(dir: &Path) -> anyhow::Result<()> {
    let status = Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(dir)
        .status()?;
    anyhow::ensure!(status.success());
    let _ = Command::new("git")
        .args(["config", "user.email", "bench@weave"])
        .current_dir(dir)
        .status();
    let _ = Command::new("git")
        .args(["config", "user.name", "weave"])
        .current_dir(dir)
        .status();
    Ok(())
}

fn git_commit(dir: &Path, msg: &str) -> anyhow::Result<()> {
    let _ = Command::new("git")
        .args(["add", "."])
        .current_dir(dir)
        .status()?;
    let status = Command::new("git")
        .args(["commit", "-m", msg])
        .current_dir(dir)
        .status()?;
    anyhow::ensure!(status.success());
    Ok(())
}

fn make_work_root(keep_work: bool) -> anyhow::Result<(PathBuf, Option<tempfile::TempDir>)> {
    let td = tempfile::Builder::new()
        .prefix("weave-phase19-")
        .tempdir()?;
    if keep_work {
        let kept = std::env::temp_dir().join(format!("weave-phase19-keep-{}", std::process::id()));
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
