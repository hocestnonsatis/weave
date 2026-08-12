//! Orchestration engine for Weave.
//!
//! Coordinates Git discovery, project layout, local `.weave/` metadata, the
//! global content store, artifact acquisition, materialization, and activation.

#![deny(missing_docs)]

mod acquire;
mod adoption;
mod config;
mod doctor;
mod env_cmd;
mod environment;
mod exec;
mod exec_discover;
mod exec_plan;
mod gc;
mod guide;
mod hash_artifact;
mod init;
mod policy_pack;
mod prebuild_fetch;
mod prebuild_resolve;
mod project;
mod recover;
mod registry;
mod status;
mod switch;

#[cfg(test)]
mod test_util;

pub use acquire::{
    acquire_one, prepare_artifacts, prepare_artifacts_for_platform, prepare_lockfile_artifacts,
    AcquireFilterReport, ArtifactRequest, ArtifactSource, DefaultArtifactSource, FetchedArtifact,
    FileArtifactSource, HttpArtifactSource, StoredArtifact,
};
pub use adoption::{
    assess_adoption, render_adoption_text, AdoptionAction, AdoptionAssessment, AdoptionPackageGap,
    AdoptionVerdict,
};
pub use config::{
    ExecutionConfig, PrebuildConfig, PrebuildFetchSpec, ProjectConfig, WEAVE_CONFIG_VERSION,
};
pub use doctor::{doctor_project, DoctorFinding, DoctorReport, DoctorSeverity};
pub use env_cmd::{
    env_create, env_create_with_opts, env_list, env_list_entries, env_list_filtered, env_prune,
    env_remove, env_show, EnvCreateOpts, EnvListEntry, EnvPruneOpts, EnvPruneReport,
    EnvRemoveReport,
};
pub use environment::{
    create_environment, create_environment_with_opts, CreateEnvironmentOpts, EnvironmentId,
    EnvironmentRecord, EnvironmentStore, PlatformIdentity,
};
pub use exec::{
    apply_sealed_outputs, build_exec_identity, bwrap_available, bwrap_bin, digest_tree,
    digest_tree_excluding, discover_policies_for_project, ensure_package_outputs_on_candidate,
    exec_plan_for_project, exec_plan_with_adoption, exec_run_sandboxed,
    integrate_execution_into_candidate, lookup_exec_cache, persist_exec_cache, probe_node_identity,
    refuse_live_node_modules, require_execution_enabled, require_sandbox, runnable_entries,
    seal_declared_outputs, validate_declared_output, verify_exec_cache_hit, ExecCacheRecord,
    ExecIdentity, ExecIntegrateReport, ExecRunReport, ExecRunRequest, ExecSealReport,
};
pub use exec_discover::{
    discover_package_dir, merge_suggestion_into_config, resolve_package_dir_for_discovery,
    suggest_execution_policy, suggest_execution_policy_with_prebuilds,
    validate_output_candidate_path, BlockedPackage, DiscoveredScript, OutputCandidate,
    OutputCandidateSource, PackageDiscovery, PolicyReviewStatus, SuggestedExecutionPolicy,
};
pub use exec_plan::{
    plan_execution, plan_execution_at, plan_execution_with_config, ExecNeedClass, ExecPlan,
    ExecPlanEntry, ExecSandboxProfile,
};
pub use gc::{
    gc_project, gc_project_with_options, gc_store, gc_store_with_roots, GcOptions, GcReport,
};
pub use guide::{adoption_guide, render_adoption_guide, AdoptionGuide};
pub use hash_artifact::{hash_verified_artifact, HashArtifactReport, HashArtifactRequest};
pub use init::{init_project, InitOutcome};
pub use policy_pack::{
    apply_policy_pack, load_policy_pack, render_policy_pack_toml, validate_policy_pack, PolicyPack,
    PolicyPackApplyReport, POLICY_PACK_VERSION,
};
pub use prebuild_fetch::{
    ensure_prebuild_on_candidate, https_fetch_allowlisted, plan_all_prebuilds,
    plan_prebuild_for_package, select_prebuild_spec, validate_fetch_url, MockPrebuildTransport,
    PrebuildEnsureReport, PrebuildHttpResponse, PrebuildPlanEntry, PrebuildProvenance,
    PrebuildTransport, UreqPrebuildTransport,
};
pub use prebuild_resolve::{
    render_prebuild_suggestion_toml, resolve_native_prebuilds, resolve_native_prebuilds_at,
    suggestable_prebuild_fetches, NativePrebuildReport, NativePrebuildRequirement,
    PrebuildPatternKind, PrebuildResolveStatus, SuggestedPrebuildFetch,
};
pub use project::discover_project;
pub use recover::{recover_project, RecoverOpts, RecoverReport};
pub use status::project_status;
pub use switch::{
    materialize_project, materialize_project_with_options, materialize_project_with_source,
    materialize_project_with_source_options, switch_project, switch_project_with_options,
    switch_project_with_source, switch_project_with_source_options, PrepareOutcome, SwitchOptions,
    SwitchOutcome,
};
