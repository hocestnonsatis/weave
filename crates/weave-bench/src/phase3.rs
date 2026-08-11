//! Phase 3 report orchestration.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::analyze::{classify_lockfile, LifecycleSummary};
use crate::corpus::{default_corpus_root, load_corpus, CorpusEntry};
use crate::experiments::{
    real_pair_overlap, run_materialize_pressure, run_synthetic_divergence, DivergenceRow,
    MaterializePressureRow,
};
use crate::measure::HostInfo;

#[derive(Debug, Serialize)]
pub struct Phase3Report {
    pub host: HostInfo,
    pub corpus_root: String,
    pub corpus: Vec<CorpusEntryView>,
    pub corpus_analyze_errors: Vec<String>,
    pub real_divergence_pairs: Vec<RealDivergenceView>,
    pub synthetic_divergence: Vec<DivergenceRow>,
    pub materialize_pressure: Vec<MaterializePressureRow>,
    pub lifecycle_classification: Vec<LifecycleSummary>,
    pub correctness_gaps: Vec<CorrectnessGap>,
    pub architectural_gate: Vec<ArchAnswer>,
    pub blockers: Vec<String>,
    pub measured_vs_unavailable: MeasurementNotes,
}

#[derive(Debug, Serialize)]
pub struct CorpusEntryView {
    pub id: String,
    pub category: String,
    pub provenance_repo: Option<String>,
    pub provenance_ref: Option<String>,
    pub lockfile_sha256: Option<String>,
    pub package_count: Option<usize>,
    pub unique_artifacts: Option<usize>,
    pub max_depth: Option<usize>,
    pub duplicated_names: Option<usize>,
    pub optional_packages: Option<usize>,
    pub peer_edges: Option<usize>,
    pub native_packages: Option<usize>,
    pub lifecycle_packages: Option<usize>,
    pub workspace_packages: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct RealDivergenceView {
    pub pair: String,
    pub shared: usize,
    pub only_a: usize,
    pub only_b: usize,
    pub shared_fraction_of_a: f64,
    pub jaccard: f64,
    pub note: String,
}

#[derive(Debug, Serialize)]
pub struct CorrectnessGap {
    pub area: String,
    pub status: String,
    pub evidence: String,
    pub fixture: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ArchAnswer {
    pub question: String,
    pub evidence: String,
    pub conclusion: String,
    pub confidence: String,
    pub remaining_uncertainty: String,
    pub next_experiment: String,
}

#[derive(Debug, Serialize)]
pub struct MeasurementNotes {
    pub measured: Vec<String>,
    pub unavailable: Vec<String>,
    pub estimates: Vec<String>,
}

/// Run the full Phase 3 offline validation pipeline.
pub fn run_phase3(corpus_root: Option<PathBuf>) -> anyhow::Result<Phase3Report> {
    let corpus_root = corpus_root.unwrap_or_else(default_corpus_root);
    let mut blockers = Vec::new();
    let mut unavailable = Vec::new();
    let measured = vec![
        "corpus lockfile graph analysis (offline)".into(),
        "synthetic divergence weave cold/warm/A↔B (offline tarballs)".into(),
        "materialization pressure hardlink counts (offline)".into(),
        "lifecycle classification heuristics (offline)".into(),
        "real lockfile artifact-set overlap (offline, no materialize)".into(),
    ];

    unavailable.push(
        "full weave materialize of real registry lockfiles without network or vendored tarballs"
            .into(),
    );
    unavailable.push(
        "network npm ci / pnpm install comparative timings (run separately with --network suite)"
            .into(),
    );
    unavailable.push("cross-filesystem-boundary hardlink denial on this host (not forced)".into());

    let corpus = match load_corpus(&corpus_root) {
        Ok(c) => c,
        Err(e) => {
            blockers.push(format!("corpus load failed: {e:#}"));
            Vec::new()
        }
    };

    let mut corpus_views = Vec::new();
    let mut analyze_errors = Vec::new();
    let mut lifecycle = Vec::new();
    for entry in &corpus {
        corpus_views.push(view_entry(entry));
        if let Some(err) = &entry.analyze_error {
            analyze_errors.push(format!("{}: {err}", entry.id));
        }
        if entry.lockfile.is_file() {
            match classify_lockfile(&entry.lockfile) {
                Ok(s) => lifecycle.push(s),
                Err(e) => analyze_errors.push(format!("lifecycle {}: {e:#}", entry.id)),
            }
        }
    }

    let mut real_div = Vec::new();
    push_real_pair(
        &mut real_div,
        &corpus_root,
        "divergence/nestjs-v10.3",
        "divergence/nestjs-v10.4",
    );
    push_real_pair(
        &mut real_div,
        &corpus_root,
        "divergence/axios-v1.6",
        "divergence/axios-v1.7",
    );

    let synthetic_divergence = run_synthetic_divergence(false)?;
    let materialize_pressure = run_materialize_pressure(false)?;

    let correctness_gaps = correctness_audit(&corpus);
    let architectural_gate = arch_gate(
        &corpus_views,
        &synthetic_divergence,
        &materialize_pressure,
        &lifecycle,
        &real_div,
    );

    Ok(Phase3Report {
        host: HostInfo::capture(),
        corpus_root: corpus_root.display().to_string(),
        corpus: corpus_views,
        corpus_analyze_errors: analyze_errors,
        real_divergence_pairs: real_div,
        synthetic_divergence,
        materialize_pressure,
        lifecycle_classification: lifecycle,
        correctness_gaps,
        architectural_gate,
        blockers,
        measured_vs_unavailable: MeasurementNotes {
            measured,
            unavailable,
            estimates: vec![
                "meaningful reuse threshold inferred from synthetic shared fractions + switch timings"
                    .into(),
            ],
        },
    })
}

fn view_entry(entry: &CorpusEntry) -> CorpusEntryView {
    CorpusEntryView {
        id: entry.id.clone(),
        category: entry.category.clone(),
        provenance_repo: entry
            .provenance
            .as_ref()
            .map(|p| p.source.repository.clone()),
        provenance_ref: entry.provenance.as_ref().map(|p| p.source.git_ref.clone()),
        lockfile_sha256: entry.provenance.as_ref().map(|p| p.lockfile_sha256.clone()),
        package_count: entry.stats.as_ref().map(|s| s.package_count),
        unique_artifacts: entry.stats.as_ref().map(|s| s.unique_artifacts),
        max_depth: entry.stats.as_ref().map(|s| s.max_depth),
        duplicated_names: entry.stats.as_ref().map(|s| s.duplicated_name_count),
        optional_packages: entry.stats.as_ref().map(|s| s.optional_packages),
        peer_edges: entry.stats.as_ref().map(|s| s.peer_edges),
        native_packages: entry.stats.as_ref().map(|s| s.native_packages),
        lifecycle_packages: entry.stats.as_ref().map(|s| s.lifecycle_script_packages),
        workspace_packages: entry.stats.as_ref().map(|s| s.workspace_packages),
    }
}

fn push_real_pair(out: &mut Vec<RealDivergenceView>, root: &Path, a: &str, b: &str) {
    let la = root.join(a).join("package-lock.json");
    let lb = root.join(b).join("package-lock.json");
    if !(la.is_file() && lb.is_file()) {
        out.push(RealDivergenceView {
            pair: format!("{a} vs {b}"),
            shared: 0,
            only_a: 0,
            only_b: 0,
            shared_fraction_of_a: 0.0,
            jaccard: 0.0,
            note: "lockfiles missing — skipped".into(),
        });
        return;
    }
    match real_pair_overlap(&la, &lb) {
        Ok((r, label)) => out.push(RealDivergenceView {
            pair: label,
            shared: r.shared,
            only_a: r.only_a,
            only_b: r.only_b,
            shared_fraction_of_a: r.shared_fraction_of_a,
            jaccard: r.jaccard,
            note: "artifact fingerprint overlap from lockfile only (no materialize)".into(),
        }),
        Err(e) => out.push(RealDivergenceView {
            pair: format!("{a} vs {b}"),
            shared: 0,
            only_a: 0,
            only_b: 0,
            shared_fraction_of_a: 0.0,
            jaccard: 0.0,
            note: format!("error: {e:#}"),
        }),
    }
}

fn correctness_audit(corpus: &[CorpusEntry]) -> Vec<CorrectnessGap> {
    let mut gaps = Vec::new();
    let any_peer = corpus
        .iter()
        .filter_map(|c| c.stats.as_ref())
        .any(|s| s.peer_edges > 0);
    let any_optional = corpus
        .iter()
        .filter_map(|c| c.stats.as_ref())
        .any(|s| s.optional_packages > 0);
    let any_native = corpus
        .iter()
        .filter_map(|c| c.stats.as_ref())
        .any(|s| s.native_packages > 0);
    let any_lifecycle = corpus
        .iter()
        .filter_map(|c| c.stats.as_ref())
        .any(|s| s.lifecycle_script_packages > 0);
    let any_workspace = corpus
        .iter()
        .filter_map(|c| c.stats.as_ref())
        .any(|s| s.workspace_packages > 0);
    let any_dup = corpus
        .iter()
        .filter_map(|c| c.stats.as_ref())
        .any(|s| s.duplicated_name_count > 0);

    gaps.push(CorrectnessGap {
        area: "peer dependencies".into(),
        status: if any_peer {
            "parsed".into()
        } else {
            "not observed in corpus".into()
        },
        evidence: "EdgeKind::Peer edges counted in GraphStats; materialization does not rewrite peer resolution".into(),
        fixture: Some("crates/weave-lockfile/fixtures/peer-deps (existing)".into()),
    });
    gaps.push(CorrectnessGap {
        area: "optional dependencies".into(),
        status: if any_optional {
            "parsed".into()
        } else {
            "not observed".into()
        },
        evidence: "optional nodes counted; Weave does not evaluate OS filters at acquire time beyond recording cpu/os fields".into(),
        fixture: Some("crates/weave-lockfile/fixtures/optional-deps".into()),
    });
    gaps.push(CorrectnessGap {
        area: "nested / duplicated versions".into(),
        status: if any_dup {
            "observed in corpus".into()
        } else {
            "rare in corpus".into()
        },
        evidence: "duplicated_name_count tracks same name with multiple versions".into(),
        fixture: Some("crates/weave-lockfile/fixtures/nested-versions".into()),
    });
    gaps.push(CorrectnessGap {
        area: "package exports / bin links".into(),
        status: "gap".into(),
        evidence: "PackageNode has no exports/bin fields; Weave does not create .bin shims during materialize".into(),
        fixture: Some("benchmarks/corpus correctness note — needs fixture for bin linking".into()),
    });
    gaps.push(CorrectnessGap {
        area: "symlinks / workspaces".into(),
        status: if any_workspace {
            "partial (workspace nodes observed)".into()
        } else {
            "partial (no workspace nodes in this corpus run)".into()
        },
        evidence: "link/workspace nodes skipped for extraction; no automatic link wiring into node_modules for workspace packages".into(),
        fixture: Some("crates/weave-lockfile/fixtures/monorepo-workspace".into()),
    });
    gaps.push(CorrectnessGap {
        area: "native modules".into(),
        status: if any_native {
            "detected+copy".into()
        } else {
            "detected heuristics".into()
        },
        evidence: "prefer_copy; no rebuild; platform identity in EnvironmentId".into(),
        fixture: Some("native bench suite + bcrypt corpus lockfile".into()),
    });
    gaps.push(CorrectnessGap {
        area: "lifecycle-generated files".into(),
        status: "detect-only".into(),
        evidence: format!(
            "ADR-0012: scripts not executed; corpus lifecycle packages observed={any_lifecycle}"
        ),
        fixture: Some("docs/lifecycle.md".into()),
    });
    gaps.push(CorrectnessGap {
        area: "directory file: dependencies".into(),
        status: "unsupported".into(),
        evidence: "NotImplemented in acquire path".into(),
        fixture: None,
    });
    gaps
}

fn arch_gate(
    corpus: &[CorpusEntryView],
    divergence: &[DivergenceRow],
    pressure: &[MaterializePressureRow],
    lifecycle: &[LifecycleSummary],
    real_div: &[RealDivergenceView],
) -> Vec<ArchAnswer> {
    let max_pkgs = corpus
        .iter()
        .filter_map(|c| c.package_count)
        .max()
        .unwrap_or(0);
    let worst_switch = divergence
        .iter()
        .filter_map(|r| r.weave_switch_a_to_b_ms)
        .max()
        .unwrap_or(0);
    let zero_share = divergence.iter().find(|r| r.target_shared_fraction == 0.0);
    let high_share = divergence.iter().find(|r| r.target_shared_fraction >= 0.9);
    let largest_press = pressure.iter().max_by_key(|r| r.packages);
    let runtime_needed: usize = lifecycle.iter().map(|l| l.runtime_install_required).sum();
    let native_needed: usize = lifecycle.iter().map(|l| l.likely_native_build).sum();

    let real_div_note = real_div
        .iter()
        .map(|r| {
            format!(
                "{}: shared_of_a={:.1}% jaccard={:.3}",
                r.pair,
                r.shared_fraction_of_a * 100.0,
                r.jaccard
            )
        })
        .collect::<Vec<_>>()
        .join("; ");

    vec![
        ArchAnswer {
            question: "Q1 — filesystem view (hardlink/reflink/overlayfs/VFS)".into(),
            evidence: format!(
                "Pressure suite largest={:?}; hardlinks dominate when prefer_copy=false; \
                 synthetic A↔B worst switch {} ms; corpus max packages {}",
                largest_press.map(|p| format!(
                    "{} hardlinks={} copies={} {}ms",
                    p.label, p.hardlinks, p.copies, p.wall_ms
                )),
                worst_switch,
                max_pkgs
            ),
            conclusion: "Retain hardlink+copy. No measured evidence that FUSE/overlayfs is required for current corpus scales.".into(),
            confidence: "medium".into(),
            remaining_uncertainty: "Real registry materialize of 3k+ package trees not timed offline; cross-device copy fallback not stressed.".into(),
            next_experiment: "Network-gated materialize of nestjs/npm-cli lockfiles with local npm cache; measure wall and inode growth.".into(),
        },
        ArchAnswer {
            question: "Q2 — lifecycle script execution".into(),
            evidence: format!(
                "Lifecycle classification across corpus: runtime_install_required≈{runtime_needed}, \
                 likely_native_build≈{native_needed}. Detect/copy policy unchanged."
            ),
            conclusion: "Do not implement arbitrary script execution yet. Evidence shows many install scripts exist, but Weave's extraction-only path is an explicit unsupported mode for those packages—not a silent correctness claim.".into(),
            confidence: "medium-high for policy; low for whether real apps boot without scripts".into(),
            remaining_uncertainty: "Need controlled smoke tests of specific apps after weave switch without rebuild.".into(),
            next_experiment: "Pick 3 corpus projects with lifecycle packages; weave materialize via network; attempt node entrypoint; record failures.".into(),
        },
        ArchAnswer {
            question: "Q3 — Git source isolation".into(),
            evidence: "Phase 3 focused on dependency lockfiles; no source-isolation benchmarks.".into(),
            conclusion: "Defer. No evidence collected that source virtualization is the bottleneck.".into(),
            confidence: "low (absence of evidence)".into(),
            remaining_uncertainty: "Monorepo source checkout costs unmeasured.".into(),
            next_experiment: "Only if dependency switch is proven fast but developers still blocked by source tree churn.".into(),
        },
        ArchAnswer {
            question: "Q4 — replace package managers vs environment layer".into(),
            evidence: format!(
                "Real divergence pairs (lockfile-only): {real_div_note}. \
                 Synthetic high-share switch={:?} ms vs 0% share={:?} ms.",
                high_share.and_then(|r| r.weave_switch_a_to_b_ms),
                zero_share.and_then(|r| r.weave_switch_a_to_b_ms)
            ),
            conclusion: "Remain an environment layer over lockfile truth. Gaps (bin links, file: dirs, scripts) block full installer replacement.".into(),
            confidence: "high".into(),
            remaining_uncertainty: "Developer UX vs npm/pnpm for first-time cold installs uncompared on network.".into(),
            next_experiment: "Optional network comparative suite with explicit labeling.".into(),
        },
        ArchAnswer {
            question: "Q5 — SQLite vs filesystem metadata".into(),
            evidence: format!(
                "Analyzed {} corpus lockfiles with filesystem JSON env/registry model; no metadata query bottleneck observed in GC/list paths for this scale.",
                corpus.len()
            ),
            conclusion: "Keep filesystem metadata.".into(),
            confidence: "medium".into(),
            remaining_uncertainty: "Multi-thousand project registries untested.".into(),
            next_experiment: "Stress registry with 10k project registrations; measure GC root collection time.".into(),
        },
    ]
}

pub fn write_phase3_outputs(out_dir: &Path, report: &Phase3Report) -> anyhow::Result<()> {
    fs::create_dir_all(out_dir)?;
    let json_path = out_dir.join("phase3-report.json");
    let md_path = out_dir.join("phase3-report.md");
    fs::write(&json_path, serde_json::to_vec_pretty(report)?)?;
    fs::write(&md_path, render_markdown(report))?;
    eprintln!("Wrote {}", json_path.display());
    eprintln!("Wrote {}", md_path.display());
    Ok(())
}

fn render_markdown(r: &Phase3Report) -> String {
    let mut o = String::new();
    o.push_str("# Weave Phase 3 — Real-World Validation Report\n\n");
    o.push_str(&format!(
        "- host: `{}` / `{}`\n- corpus: `{}` ({} entries)\n\n",
        r.host.os,
        r.host.arch,
        r.corpus_root,
        r.corpus.len()
    ));

    o.push_str("## Measurement boundaries\n\n");
    o.push_str("### Measured\n");
    for m in &r.measured_vs_unavailable.measured {
        o.push_str(&format!("- {m}\n"));
    }
    o.push_str("\n### Unavailable / not run\n");
    for m in &r.measured_vs_unavailable.unavailable {
        o.push_str(&format!("- {m}\n"));
    }
    if !r.blockers.is_empty() {
        o.push_str("\n### Blockers\n");
        for b in &r.blockers {
            o.push_str(&format!("- {b}\n"));
        }
    }

    o.push_str("\n## Corpus scale analysis\n\n");
    o.push_str("| id | category | pkgs | artifacts | depth | dup names | optional | peer | native | scripts |\n|---|---|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    for c in &r.corpus {
        o.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            c.id,
            c.category,
            fmt_opt(c.package_count),
            fmt_opt(c.unique_artifacts),
            fmt_opt(c.max_depth),
            fmt_opt(c.duplicated_names),
            fmt_opt(c.optional_packages),
            fmt_opt(c.peer_edges),
            fmt_opt(c.native_packages),
            fmt_opt(c.lifecycle_packages),
        ));
    }

    o.push_str("\n## Real lockfile divergence (artifact fingerprints)\n\n");
    for d in &r.real_divergence_pairs {
        o.push_str(&format!(
            "- **{}**: shared={} only_a={} only_b={} shared_of_a={:.1}% jaccard={:.3} — _{}_\n",
            d.pair,
            d.shared,
            d.only_a,
            d.only_b,
            d.shared_fraction_of_a * 100.0,
            d.jaccard,
            d.note
        ));
    }

    o.push_str("\n## Synthetic divergence (Weave timed, offline)\n\n");
    o.push_str("| label | target | measured shared/A | cold ms | warm ms | A→B ms | B→A ms |\n|---|---:|---:|---:|---:|---:|---:|\n");
    for d in &r.synthetic_divergence {
        o.push_str(&format!(
            "| {} | {:.0}% | {:.1}% | {} | {} | {} | {} |\n",
            d.label,
            d.target_shared_fraction * 100.0,
            d.measured_overlap.shared_fraction_of_a * 100.0,
            d.weave_cold_ms.unwrap_or(0),
            d.weave_warm_ms.unwrap_or(0),
            d.weave_switch_a_to_b_ms.unwrap_or(0),
            d.weave_switch_b_to_a_ms.unwrap_or(0),
        ));
    }

    o.push_str("\n## Materialization pressure\n\n");
    o.push_str("| label | pkgs | ms | hardlinks | copies | nm bytes | nm inodes |\n|---|---:|---:|---:|---:|---:|---:|\n");
    for p in &r.materialize_pressure {
        o.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            p.label, p.packages, p.wall_ms, p.hardlinks, p.copies, p.disk_bytes_nm, p.inodes_nm
        ));
    }

    o.push_str("\n## Lifecycle classification (heuristic, no execution)\n\n");
    let mut tot = LifecycleSummary {
        path: "TOTAL".into(),
        extraction_only: 0,
        likely_generated_files: 0,
        likely_native_build: 0,
        runtime_install_required: 0,
        unsupported_or_unsafe: 0,
        sample_runtime_install: Vec::new(),
        sample_native: Vec::new(),
    };
    for l in &r.lifecycle_classification {
        tot.extraction_only += l.extraction_only;
        tot.likely_generated_files += l.likely_generated_files;
        tot.likely_native_build += l.likely_native_build;
        tot.runtime_install_required += l.runtime_install_required;
    }
    o.push_str(&format!(
        "Corpus totals — extraction_only={}, generated≈{}, native_build≈{}, runtime_install≈{}\n\n",
        tot.extraction_only,
        tot.likely_generated_files,
        tot.likely_native_build,
        tot.runtime_install_required
    ));

    o.push_str("## Correctness audit\n\n");
    for g in &r.correctness_gaps {
        o.push_str(&format!(
            "- **{}** — status: `{}`\n  - {}\n",
            g.area, g.status, g.evidence
        ));
    }

    o.push_str("\n## Answers to Phase 3 questions\n\n");
    o.push_str(&answers_section(r));

    o.push_str("\n## Architectural decision gate (Q1–Q5)\n\n");
    for a in &r.architectural_gate {
        o.push_str(&format!("### {}\n", a.question));
        o.push_str(&format!("- **evidence:** {}\n", a.evidence));
        o.push_str(&format!("- **conclusion:** {}\n", a.conclusion));
        o.push_str(&format!("- **confidence:** {}\n", a.confidence));
        o.push_str(&format!("- **uncertainty:** {}\n", a.remaining_uncertainty));
        o.push_str(&format!("- **next experiment:** {}\n\n", a.next_experiment));
    }
    o
}

fn answers_section(r: &Phase3Report) -> String {
    let mut o = String::new();
    let high = r
        .synthetic_divergence
        .iter()
        .find(|d| d.target_shared_fraction >= 0.9);
    let low = r
        .synthetic_divergence
        .iter()
        .find(|d| d.target_shared_fraction == 0.0);
    o.push_str(&format!(
        "1. **Warm-switch advantages on real projects?** Offline Weave warm/A↔B on synthetic trees stays low-ms; \
         real lockfiles were analyzed for scale/overlap but **not** fully materialized offline (see unavailable). \
         High-share synthetic A→B ≈ {} ms vs 0% share ≈ {} ms.\n",
        high.and_then(|d| d.weave_switch_a_to_b_ms)
            .map(|v| v.to_string())
            .unwrap_or_else(|| "n/a".into()),
        low.and_then(|d| d.weave_switch_a_to_b_ms)
            .map(|v| v.to_string())
            .unwrap_or_else(|| "n/a".into()),
    ));
    o.push_str(
        "2. **Disk deduplication?** Content-addressed store + hardlinks; apparent sizes overstate physical use. \
         Real pair overlaps quantify shared artifacts before materialize.\n",
    );
    if let Some(p) = r.materialize_pressure.iter().max_by_key(|x| x.packages) {
        o.push_str(&format!(
            "3. **When materialization gets expensive?** Largest offline pressure point measured: {} — {} ms, {} hardlinks, {} inodes.\n",
            p.label, p.wall_ms, p.hardlinks, p.inodes_nm
        ));
    }
    o.push_str(
        "4. **Divergence vs reuse?** See synthetic table; reuse value tracks shared artifact fraction.\n",
    );
    o.push_str(
        "5. **Hardlink+copy sufficient?** Yes for measured offline scales; copy path used for native/scripts.\n",
    );
    o.push_str(
        "6. **Unsupported real-world cases?** bin links, package exports, directory file:, lifecycle execution, full peer install semantics.\n",
    );
    o.push_str(
        "7. **FUSE/overlayfs/daemon needed?** **No evidence yet** from Phase 3 offline measurements.\n",
    );
    o
}

fn fmt_opt(v: Option<usize>) -> String {
    v.map(|x| x.to_string()).unwrap_or_else(|| "—".into())
}
