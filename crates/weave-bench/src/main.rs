//! Weave benchmark harness (Phase 2–3).

mod analyze;
mod corpus;
mod experiments;
mod fixture;
mod measure;
mod phase3;
mod report;
mod scenarios;

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use report::{write_json, BenchSuiteResult};
use scenarios::{CompareFlags, SuiteKind};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Run {
            suite,
            with_npm,
            with_pnpm,
            json_out,
            keep_work,
        } => {
            let kind = SuiteKind::parse(&suite)?;
            let result = scenarios::run_suite(
                kind,
                CompareFlags {
                    npm: with_npm,
                    pnpm: with_pnpm,
                },
                keep_work,
            )?;
            print_human(&result);
            if let Some(path) = json_out {
                write_json(&path, &result).with_context(|| format!("write {}", path.display()))?;
                eprintln!("Wrote {}", path.display());
            }
            Ok(())
        }
        Commands::AnalyzeCorpus { corpus, json_out } => {
            let root = corpus.unwrap_or_else(corpus::default_corpus_root);
            let entries = corpus::load_corpus(&root)?;
            for e in &entries {
                match &e.stats {
                    Some(s) => println!(
                        "{}/{} pkgs={} artifacts={} depth={} native={} scripts={} dup_names={}",
                        e.category,
                        e.id,
                        s.package_count,
                        s.unique_artifacts,
                        s.max_depth,
                        s.native_packages,
                        s.lifecycle_script_packages,
                        s.duplicated_name_count
                    ),
                    None => println!(
                        "{}/{} ERROR {}",
                        e.category,
                        e.id,
                        e.analyze_error.as_deref().unwrap_or("?")
                    ),
                }
            }
            if let Some(path) = json_out {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&path, serde_json::to_vec_pretty(&entries)?)?;
                eprintln!("Wrote {}", path.display());
            }
            Ok(())
        }
        Commands::Phase3 { corpus, out_dir } => {
            let report = phase3::run_phase3(corpus)?;
            let out_dir = out_dir.unwrap_or_else(|| PathBuf::from("benchmarks/out/phase3"));
            phase3::write_phase3_outputs(&out_dir, &report)?;
            // Also copy a durable sample under docs when running from repo root.
            let docs = PathBuf::from("docs/benchmarks/phase3-report.md");
            if let Some(parent) = docs.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::copy(out_dir.join("phase3-report.md"), &docs);
            println!(
                "Phase 3 complete — {} corpus entries, {} arch answers",
                report.corpus.len(),
                report.architectural_gate.len()
            );
            Ok(())
        }
        Commands::Report { out_dir } => {
            let out_dir = out_dir.unwrap_or_else(|| PathBuf::from("benchmarks/out"));
            fs::create_dir_all(&out_dir)?;
            let result =
                scenarios::run_suite(SuiteKind::AllOffline, CompareFlags::default(), false)?;
            let json_path = out_dir.join("phase2-report.json");
            write_json(&json_path, &result)?;
            let md_path = out_dir.join("phase2-report.md");
            fs::write(&md_path, render_markdown_report(&result))?;
            println!("Wrote {}", json_path.display());
            println!("Wrote {}", md_path.display());
            print_human(&result);
            Ok(())
        }
        Commands::Methodology => {
            println!("{}", include_str!("../../../benchmarks/README.md"));
            Ok(())
        }
    }
}

fn print_human(result: &BenchSuiteResult) {
    println!("Weave benchmark suite — {}", result.suite);
    println!("host: {}  {}", result.host.os, result.host.arch);
    println!("work: {}", result.work_dir);
    println!();
    println!(
        "{:<40} {:>10} {:>12} {:>10} {:>10}",
        "scenario", "ms", "disk_bytes", "files", "inodes"
    );
    println!("{}", "-".repeat(86));
    for row in &result.rows {
        println!(
            "{:<40} {:>10} {:>12} {:>10} {:>10}",
            row.name,
            row.wall_ms,
            row.disk_bytes.unwrap_or(0),
            row.file_count.unwrap_or(0),
            row.approx_inodes.unwrap_or(0)
        );
        if let Some(note) = &row.note {
            println!("  note: {note}");
        }
    }
    println!();
    if let Some(summary) = &result.summary {
        println!("{summary}");
    }
}

fn render_markdown_report(result: &BenchSuiteResult) -> String {
    let mut out = String::new();
    out.push_str("# Weave Phase 2 benchmark report\n\n");
    out.push_str(&format!(
        "- host: `{}` / `{}`\n- suite: `{}`\n\n",
        result.host.os, result.host.arch, result.suite
    ));
    out.push_str(
        "| scenario | wall_ms | disk_bytes | files | inodes |\n|---|---:|---:|---:|---:|\n",
    );
    for row in &result.rows {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            row.name,
            row.wall_ms,
            row.disk_bytes.unwrap_or(0),
            row.file_count.unwrap_or(0),
            row.approx_inodes.unwrap_or(0)
        ));
    }
    if let Some(summary) = &result.summary {
        out.push_str("\n## Summary\n\n");
        out.push_str(summary);
        out.push('\n');
    }
    out.push_str(
        "\n## Notes\n\n- Offline Weave scenarios use synthetic `FileArtifactSource` fixtures.\n",
    );
    out.push_str("- Apparent disk size does not account for hardlink sharing across trees.\n");
    out
}

#[derive(Debug, Parser)]
#[command(
    name = "weave-bench",
    about = "Reproducible Weave benchmarks (Phase 2–3)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run a named offline/synthetic suite
    Run {
        #[arg(long, default_value = "tiny")]
        suite: String,
        #[arg(long)]
        with_npm: bool,
        #[arg(long)]
        with_pnpm: bool,
        #[arg(long)]
        json_out: Option<PathBuf>,
        #[arg(long)]
        keep_work: bool,
    },
    /// Analyze real-world corpus lockfiles (offline, no materialize)
    AnalyzeCorpus {
        #[arg(long)]
        corpus: Option<PathBuf>,
        #[arg(long)]
        json_out: Option<PathBuf>,
    },
    /// Run Phase 3 real-world validation pipeline and write report
    Phase3 {
        #[arg(long)]
        corpus: Option<PathBuf>,
        #[arg(long)]
        out_dir: Option<PathBuf>,
    },
    /// Phase 2 offline suite report
    Report {
        #[arg(long)]
        out_dir: Option<PathBuf>,
    },
    /// Print benchmark methodology
    Methodology,
}
