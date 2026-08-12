//! Weave CLI entrypoint.

use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use weave_core::Error as WeaveError;
use weave_engine::{
    adoption_guide, apply_policy_pack, doctor_project, env_create_with_opts, env_list_entries,
    env_prune, env_remove, exec_plan_with_adoption, exec_run_sandboxed, gc_project_with_options,
    hash_verified_artifact, init_project, load_policy_pack, materialize_project_with_options,
    merge_suggestion_into_config, project_status, recover_project, render_adoption_guide,
    render_adoption_text, suggest_execution_policy_with_prebuilds, switch_project_with_options,
    DoctorSeverity, EnvCreateOpts, EnvPruneOpts, ExecRunRequest, GcOptions, HashArtifactRequest,
    PackageDiscovery, ProjectConfig, RecoverOpts, SwitchOptions,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            print_error(&err);
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir().context("failed to read current directory")?;

    match cli.command {
        Commands::Init { json } => {
            let outcome = init_project(&cwd).map_err(anyhow::Error::new)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&outcome)?);
            } else if outcome.created {
                println!("Initialized Weave in {}", outcome.root.display());
                println!("  metadata: {}", outcome.weave_dir.display());
                println!("  store:    {}", outcome.store_dir.display());
                println!();
                println!("Next:");
                for step in &outcome.next_steps {
                    println!("  {step}");
                }
            } else {
                println!(
                    "Weave already initialized in {} (idempotent no-op)",
                    outcome.root.display()
                );
                println!("Next:");
                for step in &outcome.next_steps {
                    println!("  {step}");
                }
            }
            Ok(())
        }
        Commands::Guide { json } => {
            let guide = adoption_guide(Some(cwd.as_path()));
            if json {
                println!("{}", serde_json::to_string_pretty(&guide)?);
            } else {
                print!("{}", render_adoption_guide(&guide));
            }
            Ok(())
        }
        Commands::Recover { json, purge_backup } => {
            let report =
                recover_project(&cwd, &RecoverOpts { purge_backup }).map_err(anyhow::Error::new)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("Weave recover — {}", report.root.display());
                for action in &report.actions {
                    println!("  {action}");
                }
                println!();
                println!("Next:");
                for step in &report.next_steps {
                    println!("  {step}");
                }
            }
            Ok(())
        }
        Commands::Status { json } => {
            let status = project_status(&cwd).map_err(anyhow::Error::new)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                print_status_human(&status);
            }
            Ok(())
        }
        Commands::Env { command } => {
            match command {
                EnvCommands::List { json, owner } => {
                    let entries =
                        env_list_entries(&cwd, owner.as_deref()).map_err(anyhow::Error::new)?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&entries)?);
                    } else if entries.is_empty() {
                        println!("No environments yet. Run `weave env create`.");
                    } else {
                        for env in entries {
                            let label = env.label.as_deref().unwrap_or("-");
                            let owner = env.owner.as_deref().unwrap_or("-");
                            let active = if env.active { "active" } else { "-" };
                            println!(
                                "{}  pkgs={}  label={}  owner={}  {}  graph={}",
                                &env.id[..16.min(env.id.len())],
                                env.package_count,
                                label,
                                owner,
                                active,
                                &env.graph_identity[..16.min(env.graph_identity.len())]
                            );
                        }
                    }
                    Ok(())
                }
                EnvCommands::Create { json, label, owner } => {
                    let record = env_create_with_opts(&cwd, &EnvCreateOpts { label, owner })
                        .map_err(anyhow::Error::new)?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&record)?);
                    } else {
                        println!("Created environment {}", record.id);
                        println!("  packages: {}", record.package_count);
                        println!(
                            "  platform: {}/{}",
                            record.platform.os, record.platform.arch
                        );
                        if let Some(label) = &record.label {
                            println!("  label:    {label}");
                        }
                        if let Some(owner) = &record.owner {
                            println!("  owner:    {owner}");
                        }
                    }
                    Ok(())
                }
                EnvCommands::Remove { target, json } => {
                    let report = env_remove(&cwd, &target).map_err(anyhow::Error::new)?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&report)?);
                    } else {
                        println!("Removed environment {}", report.removed_id);
                        if let Some(owner) = &report.owner {
                            println!("  owner: {owner}");
                        }
                    }
                    Ok(())
                }
                EnvCommands::Prune {
                    owner,
                    older_than_secs,
                    dry_run,
                    json,
                } => {
                    let report = env_prune(
                        &cwd,
                        &EnvPruneOpts {
                            owner,
                            older_than_secs,
                            dry_run,
                        },
                    )
                    .map_err(anyhow::Error::new)?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&report)?);
                    } else {
                        let mode = if report.dry_run { "dry-run" } else { "prune" };
                        println!("Environment prune ({mode}) owner={}", report.owner);
                        println!("  removed: {}", report.removed_ids.len());
                        for id in &report.removed_ids {
                            println!("    - {id}");
                        }
                        if let Some(active) = &report.skipped_active {
                            println!("  skipped active: {active}");
                        }
                        if report.skipped_too_recent > 0 {
                            println!("  skipped too recent: {}", report.skipped_too_recent);
                        }
                        println!();
                        println!("Hint: run `weave gc` afterward to reclaim unreachable store artifacts.");
                    }
                    Ok(())
                }
            }
        }
        Commands::Switch {
            target,
            with_exec,
            json,
            owner,
        } => {
            let options = SwitchOptions { with_exec, owner };
            let outcome = switch_project_with_options(&cwd, target.as_deref(), &options)
                .map_err(anyhow::Error::new)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&switch_json(&outcome))?);
            } else {
                println!("Environment activated: {}", outcome.prepare.environment.id);
                println!("  packages:  {}", outcome.prepare.environment.package_count);
                if let Some(owner) = &outcome.prepare.environment.owner {
                    println!("  owner:     {owner}");
                }
                println!(
                    "  fetched:   {}  reused: {}",
                    outcome.prepare.fetched_artifacts, outcome.prepare.reused_artifacts
                );
                println!(
                    "  packages:  {}  cache_hit/miss: {}/{}",
                    outcome.prepare.materialize.packages_materialized,
                    outcome.prepare.materialize.cache_hits,
                    outcome.prepare.materialize.cache_misses
                );
                println!(
                    "  files:     {} hardlink  {} copy",
                    outcome.prepare.materialize.hardlinked_files,
                    outcome.prepare.materialize.copied_files
                );
                if with_exec {
                    let ex = &outcome.prepare.execution;
                    println!(
                        "  exec:      considered={} cache_hits={} executed={} applied={}",
                        ex.packages_considered, ex.cache_hits, ex.executed, ex.applied
                    );
                }
                if outcome.activation.replaced_existing {
                    println!("  replaced existing node_modules");
                } else {
                    println!("  created node_modules");
                }
            }
            Ok(())
        }
        Commands::Materialize {
            with_exec,
            json,
            owner,
        } => {
            let options = SwitchOptions { with_exec, owner };
            let outcome =
                materialize_project_with_options(&cwd, &options).map_err(anyhow::Error::new)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&materialize_json(&outcome))?
                );
            } else {
                println!("Materialized candidate for {}", outcome.environment.id);
                println!("  candidate: {}", outcome.candidate_root.display());
                println!(
                    "  packages:  {}  cache_hit/miss: {}/{}",
                    outcome.materialize.packages_materialized,
                    outcome.materialize.cache_hits,
                    outcome.materialize.cache_misses
                );
                println!(
                    "  files:     {} hardlink  {} copy",
                    outcome.materialize.hardlinked_files, outcome.materialize.copied_files
                );
                println!(
                    "  fetched:   {}  reused: {}",
                    outcome.fetched_artifacts, outcome.reused_artifacts
                );
                if with_exec {
                    let ex = &outcome.execution;
                    println!(
                        "  exec:      considered={} cache_hits={} executed={} applied={}",
                        ex.packages_considered, ex.cache_hits, ex.executed, ex.applied
                    );
                }
                println!();
                println!("Candidate is not active. Run `weave switch` to activate.");
            }
            Ok(())
        }
        Commands::Gc { dry_run, json } => {
            let report = gc_project_with_options(&cwd, &GcOptions { dry_run })
                .map_err(anyhow::Error::new)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                let mode = if report.dry_run {
                    "dry-run"
                } else {
                    "reachability"
                };
                println!("Garbage collection ({mode})");
                println!("  store:                 {}", report.store_root.display());
                println!("  root projects:         {}", report.root_projects);
                println!("  root artifacts:        {}", report.root_artifacts);
                println!("  object temps:          {}", report.removed_object_temps);
                println!(
                    "  incomplete unpacked:   {}",
                    report.removed_unpacked_incomplete
                );
                println!(
                    "  orphan ready marks:    {}",
                    report.removed_unpacked_markers
                );
                println!(
                    "  unreachable objects:   {}",
                    report.removed_unreachable_objects
                );
                println!(
                    "  unreachable unpacked:  {}",
                    report.removed_unreachable_unpacked
                );
                if report.dry_run {
                    println!();
                    println!("Dry run: unreachable objects were counted, not deleted.");
                }
            }
            Ok(())
        }
        Commands::Doctor { json } => {
            let report = doctor_project(&cwd).map_err(anyhow::Error::new)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("Weave doctor — {}", report.root.display());
                for finding in &report.findings {
                    let tag = match finding.severity {
                        DoctorSeverity::Info => "ok  ",
                        DoctorSeverity::Warn => "warn",
                        DoctorSeverity::Error => "ERR ",
                    };
                    println!("  [{tag}] {}: {}", finding.check, finding.message);
                }
                if let Some(adoption) = &report.adoption {
                    println!();
                    print!("{}", render_adoption_text(adoption));
                }
            }
            if report.has_errors() {
                anyhow::bail!("doctor found errors");
            }
            Ok(())
        }
        Commands::Exec { command } => match command {
            ExecCommands::Plan { json } => {
                let (plan, cfg, adoption) =
                    exec_plan_with_adoption(&cwd).map_err(anyhow::Error::new)?;
                if json {
                    let payload = serde_json::json!({
                        "execution_enabled": cfg.execution.is_enabled(),
                        "profile": cfg.execution.profile,
                        "allow_packages": cfg.execution.allow_packages,
                        "declared_outputs": cfg.execution.declared_outputs,
                        "plan": plan,
                        "adoption": adoption,
                    });
                    println!("{}", serde_json::to_string_pretty(&payload)?);
                } else {
                    println!("Weave exec plan (dry-run, executed={})", plan.executed);
                    println!(
                        "  config execution.enabled = {}",
                        cfg.execution.is_enabled()
                    );
                    println!("  profile: {}", cfg.execution.profile);
                    println!("  allow_packages: {:?}", cfg.execution.allow_packages);
                    println!(
                        "  needs_execution: {}  would_execute(dual-gate): {}  needs_review: {}  needs_network: {}  native_policy_gaps: {}",
                        plan.needs_execution_count,
                        plan.would_execute_count,
                        plan.needs_review_count,
                        plan.needs_network_count,
                        plan.native_policy_gap_count
                    );
                    for e in &plan.entries {
                        if !e.needs_execution
                            && e.class == weave_engine::ExecNeedClass::ExtractionOnly
                            && !e.metadata_loaded
                            && e.prebuild.is_empty()
                            && e.native_prebuilds.is_empty()
                        {
                            continue;
                        }
                        println!(
                            "  - {} ({}) class={:?} needs_execution={} would_execute={} needs_network={} policy={:?}",
                            e.name.as_deref().unwrap_or("?"),
                            e.package_key,
                            e.class,
                            e.needs_execution,
                            e.would_execute,
                            e.needs_network,
                            e.policy
                        );
                        println!("      why: {}", e.reason);
                        for pb in &e.prebuild {
                            println!(
                                "      prebuild: {} host={} allowed={} selected={} abi_match={} → {}",
                                pb.url,
                                pb.host,
                                pb.host_allowed,
                                pb.selected,
                                pb.abi_match,
                                pb.reason
                            );
                        }
                        for np in &e.native_prebuilds {
                            println!(
                                "      native: pattern={:?} status={:?} url={} output={} → {}",
                                np.pattern,
                                np.status,
                                np.url.as_deref().unwrap_or("-"),
                                np.output.as_deref().unwrap_or("-"),
                                np.reason
                            );
                        }
                        if !e.discovered_scripts.is_empty() {
                            let names: Vec<_> = e
                                .discovered_scripts
                                .iter()
                                .map(|s| s.name.as_str())
                                .collect();
                            println!("      discovered_scripts: {names:?}");
                        } else if !e.candidate_scripts.is_empty() {
                            println!("      candidate_scripts: {:?}", e.candidate_scripts);
                        }
                        let safe: Vec<_> = e
                            .discovered_output_candidates
                            .iter()
                            .filter(|c| c.safe)
                            .map(|c| c.path.as_str())
                            .collect();
                        let rejected: Vec<_> = e
                            .discovered_output_candidates
                            .iter()
                            .filter(|c| !c.safe)
                            .map(|c| {
                                format!(
                                    "{} ({})",
                                    c.path,
                                    c.reject_reason.as_deref().unwrap_or("rejected")
                                )
                            })
                            .collect();
                        if !safe.is_empty() {
                            println!("      discovered_outputs (candidates): {safe:?}");
                        }
                        if !rejected.is_empty() {
                            println!("      rejected_outputs: {rejected:?}");
                        }
                        println!(
                            "      allowed_outputs (config): {:?}  package_allowed={}",
                            e.allowed_outputs, e.package_allowed
                        );
                    }
                    println!();
                    print!("{}", render_adoption_text(&adoption));
                    if !cfg.execution.is_enabled() && adoption.execution_config_required {
                        println!(
                            "Note: execution.enabled=false (env vars cannot enable execution)."
                        );
                    }
                }
                Ok(())
            }
            ExecCommands::Suggest { json, write } => {
                let (plan, cfg, adoption) =
                    exec_plan_with_adoption(&cwd).map_err(anyhow::Error::new)?;
                let mut discoveries: Vec<PackageDiscovery> = Vec::new();
                let mut native_reports: Vec<weave_engine::NativePrebuildReport> = Vec::new();
                for e in &plan.entries {
                    if let Some(dir) = weave_engine::resolve_package_dir_for_discovery(
                        &cwd,
                        &e.package_key,
                        e.name.as_deref(),
                    ) {
                        if let Ok(d) = weave_engine::discover_package_dir(&dir) {
                            discoveries.push(d);
                        }
                    }
                    if !e.native_prebuilds.is_empty() {
                        let needs_manual_policy = e.native_prebuilds.iter().any(|r| {
                            !matches!(
                                r.status,
                                weave_engine::PrebuildResolveStatus::Configured
                                    | weave_engine::PrebuildResolveStatus::Suggestable
                            )
                        });
                        native_reports.push(weave_engine::NativePrebuildReport {
                            package: e.name.clone().unwrap_or_else(|| e.package_key.clone()),
                            version: None,
                            requirements: e.native_prebuilds.clone(),
                            needs_manual_policy,
                        });
                    }
                }
                let suggestion = suggest_execution_policy_with_prebuilds(
                    &discoveries,
                    &native_reports,
                    &cfg.execution,
                );
                if json {
                    let payload = serde_json::json!({
                        "suggestion": suggestion,
                        "adoption": adoption,
                    });
                    println!("{}", serde_json::to_string_pretty(&payload)?);
                } else {
                    println!("Weave exec suggest (review only — does not enable execution)");
                    println!("  packages suggested: {:?}", suggestion.allow_packages);
                    if !suggestion.blocked_packages.is_empty() {
                        println!("  blocked (never auto-approved):");
                        for b in &suggestion.blocked_packages {
                            println!("    - {}: {}", b.name, b.reason);
                        }
                    }
                    if !suggestion.incomplete_packages.is_empty() {
                        println!(
                            "  incomplete (need exact outputs): {:?}",
                            suggestion.incomplete_packages
                        );
                    }
                    if !suggestion.native_policy_gaps.is_empty() {
                        println!(
                            "  native policy gaps (manual prebuild.fetches / SRI): {:?}",
                            suggestion.native_policy_gaps
                        );
                        println!(
                            "  → Weave will not invent SRI/URLs; verify artifacts and declare \
                             fetches manually (docs/adoption.md)."
                        );
                    }
                    if !suggestion.prebuild_fetches.is_empty() {
                        println!(
                            "  reviewable prebuild drafts: {} (never auto-approved)",
                            suggestion.prebuild_fetches.len()
                        );
                    }
                    println!();
                    println!("{}", suggestion.toml_fragment);
                    println!();
                    print!("{}", render_adoption_text(&adoption));
                }
                if write {
                    let was_enabled = cfg.execution.enabled;
                    let mut new_cfg = cfg.clone();
                    merge_suggestion_into_config(&mut new_cfg.execution, &suggestion);
                    // Hard guarantee: suggestions never enable execution.
                    new_cfg.execution.enabled = was_enabled;
                    let path = cwd.join(".weave/config.toml");
                    std::fs::write(&path, new_cfg.to_toml_string().map_err(anyhow::Error::new)?)
                        .with_context(|| format!("write {}", path.display()))?;
                    println!(
                        "Wrote reviewed allowlists to {} (enabled={})",
                        path.display(),
                        new_cfg.execution.enabled
                    );
                    println!("Reminder: plain switch stays execution-free; use --with-exec after enabling.");
                }
                Ok(())
            }
            ExecCommands::Run {
                package,
                input,
                script,
            } => {
                let report = exec_run_sandboxed(&ExecRunRequest {
                    project_root: cwd.clone(),
                    package,
                    input_package_dir: input,
                    script_rel: script,
                })
                .map_err(anyhow::Error::new)?;
                println!("Sandboxed execution ok: {}", report.package);
                println!("  work:     {}", report.work_root.display());
                println!(
                    "  sealed:   {} ({})",
                    report.seal.output_artifact_id,
                    report.seal.sealed_paths.join(", ")
                );
                println!("  cache:    {}", report.seal.cache_key);
                println!(
                    "  platform: {}/{} node_abi={}",
                    report.platform_os, report.platform_cpu, report.node_abi
                );
                Ok(())
            }
            ExecCommands::HashArtifact {
                path,
                package,
                output,
                url,
                node_abi,
                os,
                cpu,
                json,
            } => {
                let report = hash_verified_artifact(&HashArtifactRequest {
                    path,
                    package,
                    output,
                    url,
                    node_abi,
                    os,
                    cpu,
                })
                .map_err(anyhow::Error::new)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
                    println!("Weave hash-artifact (offline measurement — not an approval)");
                    println!("  path:      {}", report.path);
                    println!("  bytes:     {}", report.size_bytes);
                    println!("  integrity: {}", report.integrity);
                    if let Some(host) = &report.host {
                        println!("  host:      {host}");
                    }
                    println!();
                    println!("{}", report.toml_fragment);
                    println!("{}", report.note);
                }
                Ok(())
            }
            ExecCommands::ApplyPack { path, write, json } => {
                let pack = load_policy_pack(&path).map_err(anyhow::Error::new)?;
                let mut cfg = ProjectConfig::load(&cwd).map_err(anyhow::Error::new)?;
                let was_enabled = cfg.execution.enabled;
                let was_profile = cfg.execution.profile.clone();
                let report = apply_policy_pack(&mut cfg, &pack);
                // Belt-and-suspenders.
                cfg.execution.enabled = was_enabled;
                cfg.execution.profile = was_profile;
                if json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
                    println!("Weave exec apply-pack (review merge — does not enable execution)");
                    println!("  pack:           {}", report.pack_id);
                    println!("  hosts added:    {:?}", report.hosts_added);
                    println!("  fetches added:  {}", report.fetches_added);
                    println!("  packages added: {:?}", report.packages_added);
                    println!(
                        "  enabled left:   {}  profile left: {}",
                        cfg.execution.enabled, cfg.execution.profile
                    );
                    println!("{}", report.note);
                }
                if write {
                    let out = cwd.join(".weave/config.toml");
                    std::fs::write(&out, cfg.to_toml_string().map_err(anyhow::Error::new)?)
                        .with_context(|| format!("write {}", out.display()))?;
                    println!(
                        "Wrote merge to {} (enabled={})",
                        out.display(),
                        cfg.execution.enabled
                    );
                } else if !json {
                    println!("Dry-run only. Re-run with --write after reviewing the pack.");
                }
                Ok(())
            }
        },
    }
}

fn print_status_human(status: &weave_core::ProjectStatus) {
    println!("Weave status");
    println!("------------");
    println!(
        "Initialized: {}",
        if status.initialized { "yes" } else { "no" }
    );
    println!();
    println!("Git");
    println!("  root:   {}", status.git.root);
    println!(
        "  branch: {}",
        status.git.branch.as_deref().unwrap_or("(detached HEAD)")
    );
    println!("  head:   {}", status.git.head);
    println!("  dirty:  {}", if status.git.dirty { "yes" } else { "no" });
    println!(
        "  dependency files dirty: {}",
        if status.git.dependency_files_dirty {
            "yes"
        } else {
            "no"
        }
    );
    println!();
    println!("Dependency");
    println!(
        "  package.json: {}",
        if status.dependency.package_json {
            "present"
        } else {
            "missing"
        }
    );
    match (
        status.dependency.lockfile_present,
        &status.dependency.lockfile_kind,
        &status.dependency.lockfile_path,
    ) {
        (true, Some(kind), Some(path)) => {
            println!("  lockfile:     {} ({})", path, kind.as_str());
        }
        (true, _, Some(path)) => println!("  lockfile:     {path}"),
        _ => println!("  lockfile:     missing (npm package-lock.json required)"),
    }
    if let Some(count) = status.dependency.package_count {
        println!("  packages:     {count}");
    }
    if let Some(id) = &status.dependency.graph_identity {
        let short = if id.len() > 16 {
            &id[..16]
        } else {
            id.as_str()
        };
        println!("  graph id:     {short}…");
    }
    if let Some(err) = &status.dependency.parse_error {
        println!("  parse error:  {err}");
    }
    println!();
    println!("Materialization");
    println!(
        "  node_modules: {}",
        if status.materialization.node_modules_present {
            "present"
        } else {
            "absent"
        }
    );
    println!(
        "  active env:   {}",
        status
            .materialization
            .active_environment
            .as_deref()
            .unwrap_or("(none)")
    );
    println!();
    println!("Environments");
    println!("  known: {}", status.environment.known_count);
    for env in &status.environment.environments {
        let mark = if env.active { "*" } else { " " };
        let owner = env.owner.as_deref().unwrap_or("-");
        let label = env.label.as_deref().unwrap_or("-");
        println!(
            "  {mark} {}  label={label}  owner={owner}  pkgs={}",
            &env.id[..12.min(env.id.len())],
            env.package_count
        );
    }
    if !status.next_steps.is_empty() {
        println!();
        println!("Next steps");
        for step in &status.next_steps {
            println!("  - {step}");
        }
    }
    if !status.initialized {
        println!();
        println!("Hint: run `weave guide` then `weave init --json`.");
    }
}

fn switch_json(outcome: &weave_engine::SwitchOutcome) -> serde_json::Value {
    let env = &outcome.prepare.environment;
    let m = &outcome.prepare.materialize;
    serde_json::json!({
        "ok": true,
        "operation": "switch",
        "environment": {
            "id": env.id.as_str(),
            "label": env.label,
            "owner": env.owner,
            "package_count": env.package_count,
            "graph_identity": env.graph_identity.as_str(),
            "created_at": env.created_at,
            "last_activated_at": env.last_activated_at,
        },
        "acquire": {
            "fetched": outcome.prepare.fetched_artifacts,
            "reused": outcome.prepare.reused_artifacts,
        },
        "materialize": {
            "packages": m.packages_materialized,
            "cache_hits": m.cache_hits,
            "cache_misses": m.cache_misses,
            "hardlinked_files": m.hardlinked_files,
            "copied_files": m.copied_files,
        },
        "activation": {
            "node_modules": outcome.activation.node_modules,
            "replaced_existing": outcome.activation.replaced_existing,
        },
        "execution": {
            "considered": outcome.prepare.execution.packages_considered,
            "cache_hits": outcome.prepare.execution.cache_hits,
            "executed": outcome.prepare.execution.executed,
            "applied": outcome.prepare.execution.applied,
        }
    })
}

fn materialize_json(outcome: &weave_engine::PrepareOutcome) -> serde_json::Value {
    let env = &outcome.environment;
    let m = &outcome.materialize;
    serde_json::json!({
        "ok": true,
        "operation": "materialize",
        "activated": false,
        "candidate_root": outcome.candidate_root,
        "environment": {
            "id": env.id.as_str(),
            "label": env.label,
            "owner": env.owner,
            "package_count": env.package_count,
            "graph_identity": env.graph_identity.as_str(),
        },
        "acquire": {
            "fetched": outcome.fetched_artifacts,
            "reused": outcome.reused_artifacts,
        },
        "materialize": {
            "packages": m.packages_materialized,
            "cache_hits": m.cache_hits,
            "cache_misses": m.cache_misses,
            "hardlinked_files": m.hardlinked_files,
            "copied_files": m.copied_files,
        }
    })
}

fn print_error(err: &anyhow::Error) {
    eprintln!("error: {err}");
    for cause in err.chain().skip(1) {
        eprintln!("  caused by: {cause}");
    }

    if let Some(weave_err) = err.downcast_ref::<WeaveError>() {
        if let Some(hint) = recovery_hint(weave_err) {
            eprintln!();
            eprintln!("{hint}");
        }
    }
}

fn recovery_hint(err: &WeaveError) -> Option<String> {
    let hint = match err {
        WeaveError::NotAGitRepository { .. } => {
            "Preserved state: none changed.\nRetry: run from inside a Git repository."
        }
        WeaveError::MissingPackageJson { .. } => {
            "Preserved state: none changed.\nRetry: add package.json at the repository root."
        }
        WeaveError::MissingLockfile { .. } => {
            "Preserved state: none changed.\n\
             Retry: npm install (or npm i --package-lock-only), then weave init --json.\n\
             Guide: weave guide --json"
        }
        WeaveError::UnsupportedLockfile { .. } => {
            "Preserved state: none changed.\n\
             Weave supports npm package-lock.json only and will not convert Yarn/pnpm/Bun lockfiles.\n\
             Stay on your current package manager, or add package-lock.json deliberately.\n\
             Guide: weave guide --json"
        }
        WeaveError::AlreadyInitialized { .. } => {
            "Preserved state: existing .weave/ left untouched.\n\
             Note: `weave init` is idempotent — re-run is a no-op.\n\
             Next: weave doctor --json && weave switch --json"
        }
        WeaveError::NotInitialized { .. } => {
            "Preserved state: none changed.\nRetry: weave init --json\nDiagnostic: weave status --json\nGuide: weave guide --json"
        }
        WeaveError::NotImplemented(_) => {
            "Preserved state: none changed.\nThis command will arrive in a later milestone."
        }
        WeaveError::IntegrityCheckFailed { .. } => {
            "Preserved state: previous environment left unchanged.\n\
             Retry after fixing the lockfile integrity or re-acquiring the artifact."
        }
        WeaveError::ArtifactHashMismatch { .. } | WeaveError::CorruptArtifact { .. } => {
            "Preserved state: previous environment left unchanged.\n\
             Diagnostic: weave doctor\nRetry: weave gc --dry-run, then re-run weave switch."
        }
        WeaveError::InvalidState { reason, .. } => {
            if reason.contains("with-exec") || reason.contains("enabled") {
                return Some(
                    "Preserved state: previous environment left unchanged.\n\
                     Dual gate: set execution.enabled=true in .weave/config.toml after review, \
                     then weave switch --with-exec.\nDocs: docs/adoption.md"
                        .into(),
                );
            }
            if reason.contains("allow_packages") || reason.contains("declared_outputs") {
                return Some(
                    "Preserved state: previous environment left unchanged.\n\
                     Review: weave exec plan && weave exec suggest\nDocs: docs/adoption.md"
                        .into(),
                );
            }
            if reason.contains("symlink") {
                return Some(
                    "Preserved state: none changed.\n\
                     Pass a regular file path (not a symlink) to weave exec hash-artifact."
                        .into(),
                );
            }
            "Preserved state: previous Weave environment left unchanged when applicable.\n\
             Diagnostic: weave doctor\nDocs: docs/adoption.md"
        }
        _ => {
            "Preserved state: previous Weave environment left unchanged when applicable.\n\
             Diagnostic: weave doctor\nDocs: docs/adoption.md"
        }
    };
    Some(hint.into())
}

#[derive(Debug, Parser)]
#[command(
    name = "weave",
    version,
    about = "Git-aware Node.js environment engine (CAS + transactional activation)",
    long_about = "Weave materializes node_modules from a content-addressed store.\n\
                  Plain `weave switch` never runs lifecycle scripts and never opens network\n\
                  for install scripts. Experimental exec/prebuild features are opt-in only\n\
                  (execution.enabled + --with-exec) and are never silently enabled.\n\
                  Start here: `weave guide --json` (no architecture knowledge required).\n\
                  Docs: docs/agent-quickstart.md · docs/adoption.md · docs/security.md"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Print the minimal adopt/switch/recover recipe (agent-friendly)
    Guide {
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Initialize .weave/ metadata (idempotent; requires package-lock.json)
    Init {
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Show Git, dependency, and environment status
    Status {
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Clear leftover candidate / dangling active after interrupted switch
    Recover {
        /// Also delete leftover `.weave/node_modules.bak` (never touches live node_modules)
        #[arg(long)]
        purge_backup: bool,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Manage Weave environments
    Env {
        #[command(subcommand)]
        command: EnvCommands,
    },
    /// Materialize + activate node_modules from the lockfile (no scripts by default)
    Switch {
        /// Branch label or environment id prefix; omit to use current lockfile
        target: Option<String>,
        /// EXPERIMENTAL: sandboxed execution against the candidate (requires execution.enabled)
        #[arg(long)]
        with_exec: bool,
        /// Explicit owner/session id stamped on the environment (never auto-detected)
        #[arg(long)]
        owner: Option<String>,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Materialize a candidate environment without activating it
    Materialize {
        /// EXPERIMENTAL: sandboxed execution against the candidate (requires execution.enabled)
        #[arg(long)]
        with_exec: bool,
        /// Explicit owner/session id stamped on the environment (never auto-detected)
        #[arg(long)]
        owner: Option<String>,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Garbage-collect unreachable store artifacts (and incomplete temps)
    Gc {
        /// Count unreachable artifacts without deleting them
        #[arg(long)]
        dry_run: bool,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Diagnose adoption readiness and project health
    Doctor {
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// EXPERIMENTAL: opt-in sandboxed lifecycle / prebuild tools (never auto-enables)
    Exec {
        #[command(subcommand)]
        command: ExecCommands,
    },
}

#[derive(Debug, Subcommand)]
enum EnvCommands {
    /// List known environments
    List {
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
        /// Filter by exact owner/session id
        #[arg(long)]
        owner: Option<String>,
    },
    /// Create an environment from the current lockfile
    Create {
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
        /// Override label (default: current git branch)
        #[arg(long)]
        label: Option<String>,
        /// Explicit owner/session id (never auto-detected)
        #[arg(long)]
        owner: Option<String>,
    },
    /// Remove a non-active environment record (never mutates another env's node_modules)
    Remove {
        /// Environment id, id prefix, or label
        target: String,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Prune abandoned agent-owned environment records (requires --owner)
    Prune {
        /// Exact owner/session id to prune (required; never inferred)
        #[arg(long)]
        owner: String,
        /// Only prune records older than this many seconds
        #[arg(long)]
        older_than_secs: Option<u64>,
        /// Report matches without deleting
        #[arg(long)]
        dry_run: bool,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ExecCommands {
    /// Dry-run plan (never runs scripts; shows experimental gaps)
    Plan {
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Suggest allowlists / prebuild drafts (never auto-enables)
    Suggest {
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
        /// Merge suggestion into .weave/config.toml without enabling execution
        #[arg(long)]
        write: bool,
    },
    /// EXPERIMENTAL: run one allowlisted package script under Bubblewrap (offline)
    Run {
        /// Package name (must appear in execution.allow_packages)
        package: String,
        /// Path to a package directory copy (must not be live node_modules)
        #[arg(long)]
        input: std::path::PathBuf,
        /// Script path relative to the package root
        #[arg(long, default_value = "scripts/install.js")]
        script: std::path::PathBuf,
    },
    /// Hash a local file you already verified → reviewable SRI / prebuild draft (offline)
    HashArtifact {
        /// Regular file path (symlinks refused)
        path: std::path::PathBuf,
        /// Package name for the draft fetch entry
        #[arg(long)]
        package: String,
        /// Relative sealed output path under the package root
        #[arg(long)]
        output: String,
        /// Optional HTTPS URL to include in the draft (not fetched)
        #[arg(long)]
        url: Option<String>,
        /// Optional Node ABI constraint
        #[arg(long)]
        node_abi: Option<String>,
        /// Optional OS constraint (npm token, e.g. linux)
        #[arg(long)]
        os: Option<String>,
        /// Optional CPU constraint (npm token, e.g. x64)
        #[arg(long)]
        cpu: Option<String>,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Merge a reviewed policy pack into config (never enables execution)
    ApplyPack {
        /// Path to a policy pack TOML
        path: std::path::PathBuf,
        /// Write merged config to .weave/config.toml
        #[arg(long)]
        write: bool,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
}
