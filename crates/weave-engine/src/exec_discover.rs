//! Non-executing execution-policy discovery (ADR-0018 Phase 9).
//!
//! Reads package metadata and static files to propose lifecycle scripts and
//! candidate output paths. Discovery never spawns scripts, never mutates trees,
//! and never auto-approves allowlists — suggestions require human review.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::ExecutionConfig;
use crate::exec::validate_declared_output;
use crate::exec_plan::{ExecNeedClass, ExecSandboxProfile};

/// Where a candidate output path came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputCandidateSource {
    /// Parsed from `binding.gyp` / `binding.gyp.js` `target_name`.
    BindingGyp,
    /// From package.json `binary` (node-pre-gyp style).
    PackageJsonBinary,
    /// Optional author hint: `package.json` → `weave.declaredOutputs`.
    PackageWeaveHint,
    /// Well-known package name patterns (still candidates, not approvals).
    KnownPackagePattern,
    /// Conservative static string literals in install script sources.
    InstallScriptHint,
    /// Marker file present in the package tree (e.g. fixture docs).
    TreeMarker,
}

/// Review posture relative to current config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PolicyReviewStatus {
    /// No execution needed.
    ExtractionOnly,
    /// Needs execution but package/outputs are not allowlisted yet.
    NeedsReview,
    /// Allowlisted and declared outputs cover all *safe* candidates.
    Allowed,
    /// Allowlisted but declared outputs miss safe candidates (or extras only).
    PartialCoverage,
    /// Classified unsafe / ambiguous — must never be auto-approved.
    BlockedUnsafe,
    /// Needs execution but metadata could not be loaded (lockfile-only).
    MetadataMissing,
}

/// One discovered lifecycle script name from package.json.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredScript {
    /// npm lifecycle name (`install`, `postinstall`, …).
    pub name: String,
    /// Script body as declared (for display / static checks only).
    pub body: String,
    /// True when static analysis flags the body as unsafe.
    pub unsafe_body: bool,
    /// Why the body was flagged, when applicable.
    pub unsafe_reason: Option<String>,
}

/// A *candidate* output path — not an approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputCandidate {
    /// Relative path or single-segment final glob (e.g. `build/Release/*.node`).
    pub path: String,
    /// Discovery source.
    pub source: OutputCandidateSource,
    /// Passed path-safety checks as a candidate.
    pub safe: bool,
    /// Rejection reason when `safe` is false.
    pub reject_reason: Option<String>,
}

/// Package.json slice used for discovery (never executed).
#[derive(Debug, Clone, Default)]
struct PackageMeta {
    name: Option<String>,
    scripts: BTreeMap<String, String>,
    binary: Option<BinaryField>,
    weave_declared_outputs: Vec<String>,
    gypfile: bool,
}

#[derive(Debug, Clone, Default)]
struct BinaryField {
    module_name: Option<String>,
    module_path: Option<String>,
}

/// Discover scripts and output candidates from an on-disk package directory.
///
/// This function only reads files. It never runs Node or package scripts.
pub fn discover_package_dir(package_dir: &Path) -> weave_core::Result<PackageDiscovery> {
    let meta = read_package_meta(package_dir)?;
    let name = meta.name.clone().unwrap_or_else(|| {
        package_dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    });

    let mut discovered_scripts = Vec::new();
    for key in ["preinstall", "install", "postinstall", "prepare"] {
        if let Some(body) = meta.scripts.get(key) {
            let (unsafe_body, unsafe_reason) = script_body_unsafe(body);
            discovered_scripts.push(DiscoveredScript {
                name: key.to_owned(),
                body: body.clone(),
                unsafe_body,
                unsafe_reason,
            });
        }
    }

    let mut candidates: Vec<OutputCandidate> = Vec::new();

    // binding.gyp target → build/Release/<target>.node
    for gyp_name in ["binding.gyp", "binding.gyp.js"] {
        let gyp_path = package_dir.join(gyp_name);
        if gyp_path.is_file() {
            if let Some(target) =
                parse_gyp_target_name(&fs::read_to_string(&gyp_path).unwrap_or_default())
            {
                push_candidate(
                    &mut candidates,
                    format!("build/Release/{target}.node"),
                    OutputCandidateSource::BindingGyp,
                );
            } else {
                // Ambiguous native layout — surface rejected broad path.
                push_candidate(
                    &mut candidates,
                    "build/Release/*.node".into(),
                    OutputCandidateSource::BindingGyp,
                );
            }
        }
    }
    if meta.gypfile && !package_dir.join("binding.gyp").is_file() {
        push_candidate(
            &mut candidates,
            "build/Release/*.node".into(),
            OutputCandidateSource::BindingGyp,
        );
    }

    if let Some(bin) = &meta.binary {
        if let (Some(module), Some(mod_path)) = (&bin.module_name, &bin.module_path) {
            let base = mod_path.trim_start_matches("./").trim_end_matches('/');
            push_candidate(
                &mut candidates,
                format!("{base}/{module}.node"),
                OutputCandidateSource::PackageJsonBinary,
            );
        }
    }

    for hint in &meta.weave_declared_outputs {
        push_candidate(
            &mut candidates,
            hint.clone(),
            OutputCandidateSource::PackageWeaveHint,
        );
    }

    // Known name patterns (candidates only).
    let name_l = name.to_ascii_lowercase();
    if name_l == "esbuild" || name_l.starts_with("@esbuild/") {
        push_candidate(
            &mut candidates,
            "bin/esbuild".into(),
            OutputCandidateSource::KnownPackagePattern,
        );
    }
    if name_l == "exec-gen" || name_l.ends_with("/exec-gen") {
        push_candidate(
            &mut candidates,
            "generated/hello.txt".into(),
            OutputCandidateSource::KnownPackagePattern,
        );
    }
    if name_l.contains("sqlite3") {
        push_candidate(
            &mut candidates,
            "build/Release/node_sqlite3.node".into(),
            OutputCandidateSource::KnownPackagePattern,
        );
    }
    if name_l == "bcrypt" || name_l.ends_with("/bcrypt") {
        push_candidate(
            &mut candidates,
            "lib/binding/bcrypt_lib.node".into(),
            OutputCandidateSource::KnownPackagePattern,
        );
    }

    // Static string hints from discovered install-family script files / bodies.
    for script in &discovered_scripts {
        for path in static_write_hints(&script.body) {
            push_candidate(
                &mut candidates,
                path,
                OutputCandidateSource::InstallScriptHint,
            );
        }
        // If body is `node scripts/install.js`, scan that file when present.
        if let Some(rel) = script.body.strip_prefix("node ").map(str::trim) {
            let script_path = package_dir.join(rel);
            if script_path.is_file() {
                if let Ok(src) = fs::read_to_string(&script_path) {
                    for path in static_write_hints(&src) {
                        push_candidate(
                            &mut candidates,
                            path,
                            OutputCandidateSource::InstallScriptHint,
                        );
                    }
                }
            }
        }
    }
    // Always scan common install entrypoints when present (even if scripts differ).
    for rel in ["install.js", "scripts/install.js", "binding.gyp"] {
        if rel == "binding.gyp" {
            continue;
        }
        let script_path = package_dir.join(rel);
        if script_path.is_file() {
            if let Ok(src) = fs::read_to_string(&script_path) {
                for path in static_write_hints(&src) {
                    push_candidate(
                        &mut candidates,
                        path,
                        OutputCandidateSource::InstallScriptHint,
                    );
                }
            }
        }
    }

    // Dedup by path (keep first source).
    candidates.sort_by(|a, b| a.path.cmp(&b.path));
    candidates.dedup_by(|a, b| a.path == b.path);

    let unsafe_script = discovered_scripts.iter().any(|s| s.unsafe_body);
    let class = classify_discovery(&name_l, &discovered_scripts, &meta, unsafe_script);
    let sandbox = match class {
        ExecNeedClass::NativeBuild => ExecSandboxProfile::PrebuildFetch,
        _ => ExecSandboxProfile::Offline,
    };

    let blocked_unsafe = matches!(class, ExecNeedClass::UnsupportedUnsafe) || unsafe_script;
    let needs_execution = !blocked_unsafe
        && (matches!(
            class,
            ExecNeedClass::GeneratedFiles
                | ExecNeedClass::NativeBuild
                | ExecNeedClass::RuntimeInstall
        ) || !discovered_scripts.is_empty()
            || meta.gypfile
            || package_dir.join("binding.gyp").is_file()
            || package_dir.join("binding.gyp.js").is_file());

    let reason = discovery_reason(class, &discovered_scripts, &candidates, unsafe_script);

    Ok(PackageDiscovery {
        package_dir: package_dir.to_path_buf(),
        name,
        class,
        sandbox,
        needs_execution,
        discovered_scripts,
        output_candidates: candidates,
        metadata_loaded: true,
        reason,
        blocked_unsafe,
    })
}

/// Result of discovering one package directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageDiscovery {
    /// Absolute/relative package directory examined.
    pub package_dir: PathBuf,
    /// Package name.
    pub name: String,
    /// Classification.
    pub class: ExecNeedClass,
    /// Suggested sandbox profile (not a network enablement).
    pub sandbox: ExecSandboxProfile,
    /// Whether execution appears necessary for completeness.
    pub needs_execution: bool,
    /// Lifecycle scripts found in package.json.
    pub discovered_scripts: Vec<DiscoveredScript>,
    /// Output path candidates (safe and rejected).
    pub output_candidates: Vec<OutputCandidate>,
    /// True when package.json was read.
    pub metadata_loaded: bool,
    /// Human-readable rationale.
    pub reason: String,
    /// Package must never be auto-suggested into allowlists.
    pub blocked_unsafe: bool,
}

impl PackageDiscovery {
    /// Safe candidate paths suitable for a *suggested* declared_outputs list.
    ///
    /// Globs are excluded — seal requires exact files today.
    pub fn suggestable_outputs(&self) -> Vec<String> {
        self.output_candidates
            .iter()
            .filter(|c| c.safe && !c.path.contains('*'))
            .map(|c| c.path.clone())
            .collect()
    }

    /// Compare discovery against current execution config.
    pub fn review_against(&self, cfg: &ExecutionConfig) -> PolicyReviewStatus {
        if matches!(self.class, ExecNeedClass::UnsupportedUnsafe) || self.blocked_unsafe {
            return PolicyReviewStatus::BlockedUnsafe;
        }
        if !self.needs_execution {
            return PolicyReviewStatus::ExtractionOnly;
        }
        if !self.metadata_loaded {
            return PolicyReviewStatus::MetadataMissing;
        }
        if !cfg.package_allowed(&self.name) {
            return PolicyReviewStatus::NeedsReview;
        }
        let allowed: std::collections::BTreeSet<_> = cfg
            .outputs_for(&self.name)
            .iter()
            .map(|s| s.replace('\\', "/"))
            .collect();
        let needed: Vec<_> = self.suggestable_outputs();
        if needed.is_empty() {
            // Allowed but no exact outputs to declare yet — still needs review.
            return PolicyReviewStatus::NeedsReview;
        }
        if needed.iter().all(|p| allowed.contains(p)) {
            PolicyReviewStatus::Allowed
        } else {
            PolicyReviewStatus::PartialCoverage
        }
    }
}

/// Suggested `[execution]` fragment for human review (never auto-enables).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestedExecutionPolicy {
    /// Packages proposed for allow_packages (safe discoveries only).
    pub allow_packages: Vec<String>,
    /// Proposed declared_outputs (exact safe paths only).
    pub declared_outputs: BTreeMap<String, Vec<String>>,
    /// Reviewable prebuild fetch drafts (only when URL+output+SRI established).
    pub prebuild_fetches: Vec<crate::prebuild_resolve::SuggestedPrebuildFetch>,
    /// Packages discovered but blocked from suggestion.
    pub blocked_packages: Vec<BlockedPackage>,
    /// Packages needing execution but lacking exact safe outputs.
    pub incomplete_packages: Vec<String>,
    /// Packages with native download patterns that still need manual policy.
    pub native_policy_gaps: Vec<String>,
    /// Reminder: enabled must stay a human decision.
    pub enabled_suggestion: bool,
    /// Rendered TOML fragment.
    pub toml_fragment: String,
}

/// Package excluded from automatic suggestion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockedPackage {
    /// Package name.
    pub name: String,
    /// Why it was not suggested.
    pub reason: String,
}

/// Build a suggested policy from discoveries. Never sets `enabled = true`.
pub fn suggest_execution_policy(
    discoveries: &[PackageDiscovery],
    current: &ExecutionConfig,
) -> SuggestedExecutionPolicy {
    suggest_execution_policy_with_prebuilds(discoveries, &[], current)
}

/// Build a suggested policy including reviewable native prebuild drafts.
pub fn suggest_execution_policy_with_prebuilds(
    discoveries: &[PackageDiscovery],
    native_reports: &[crate::prebuild_resolve::NativePrebuildReport],
    current: &ExecutionConfig,
) -> SuggestedExecutionPolicy {
    let mut allow_packages = current.allow_packages.clone();
    let mut declared_outputs = current.declared_outputs.clone();
    let mut blocked_packages = Vec::new();
    let mut incomplete_packages = Vec::new();
    let mut native_policy_gaps = Vec::new();

    for d in discoveries {
        if d.blocked_unsafe || matches!(d.class, ExecNeedClass::UnsupportedUnsafe) {
            blocked_packages.push(BlockedPackage {
                name: d.name.clone(),
                reason: d.reason.clone(),
            });
            continue;
        }
        if !d.needs_execution {
            continue;
        }
        if d.discovered_scripts.iter().any(|s| s.unsafe_body) {
            blocked_packages.push(BlockedPackage {
                name: d.name.clone(),
                reason: "install script body flagged unsafe by static analysis".into(),
            });
            continue;
        }
        let outs = d.suggestable_outputs();
        if outs.is_empty() {
            incomplete_packages.push(d.name.clone());
            continue;
        }
        if !allow_packages.iter().any(|p| p == &d.name) {
            allow_packages.push(d.name.clone());
        }
        declared_outputs
            .entry(d.name.clone())
            .and_modify(|existing| {
                for o in &outs {
                    if !existing.iter().any(|e| e == o) {
                        existing.push(o.clone());
                    }
                }
                existing.sort();
                existing.dedup();
            })
            .or_insert_with(|| {
                let mut v = outs;
                v.sort();
                v
            });
    }
    allow_packages.sort();
    allow_packages.dedup();

    let mut prebuild_fetches = Vec::new();
    for report in native_reports {
        if report.needs_manual_policy {
            native_policy_gaps.push(report.package.clone());
        }
        for draft in crate::prebuild_resolve::suggestable_prebuild_fetches(report, current) {
            // Also ensure declared_outputs / allow_packages cover suggestable drafts.
            if !allow_packages.iter().any(|p| p == &draft.spec.package) {
                allow_packages.push(draft.spec.package.clone());
            }
            declared_outputs
                .entry(draft.spec.package.clone())
                .and_modify(|existing| {
                    if !existing.iter().any(|e| e == &draft.spec.output) {
                        existing.push(draft.spec.output.clone());
                    }
                })
                .or_insert_with(|| vec![draft.spec.output.clone()]);
            prebuild_fetches.push(draft);
        }
    }
    allow_packages.sort();
    allow_packages.dedup();
    native_policy_gaps.sort();
    native_policy_gaps.dedup();

    let mut toml_fragment = render_suggestion_toml(&allow_packages, &declared_outputs);
    let prebuild_toml = crate::prebuild_resolve::render_prebuild_suggestion_toml(
        &prebuild_fetches,
        &current.prebuild.allow_hosts,
    );
    if !prebuild_toml.is_empty() {
        toml_fragment.push('\n');
        toml_fragment.push_str(&prebuild_toml);
    }

    SuggestedExecutionPolicy {
        allow_packages,
        declared_outputs,
        prebuild_fetches,
        blocked_packages,
        incomplete_packages,
        native_policy_gaps,
        enabled_suggestion: false,
        toml_fragment,
    }
}

/// Merge a suggestion into config **without enabling execution**.
pub fn merge_suggestion_into_config(
    cfg: &mut ExecutionConfig,
    suggestion: &SuggestedExecutionPolicy,
) {
    // Never flip enabled on via suggestion.
    for pkg in &suggestion.allow_packages {
        if !cfg.allow_packages.iter().any(|p| p == pkg) {
            cfg.allow_packages.push(pkg.clone());
        }
    }
    cfg.allow_packages.sort();
    cfg.allow_packages.dedup();
    for (pkg, outs) in &suggestion.declared_outputs {
        let entry = cfg.declared_outputs.entry(pkg.clone()).or_default();
        for o in outs {
            if !entry.iter().any(|e| e == o) {
                entry.push(o.clone());
            }
        }
        entry.sort();
        entry.dedup();
    }
    // Merge reviewable prebuild drafts — still never changes profile/enabled.
    for draft in &suggestion.prebuild_fetches {
        if !cfg.prebuild.allow_hosts.iter().any(|h| h == &draft.host) {
            cfg.prebuild.allow_hosts.push(draft.host.clone());
        }
        if !cfg.prebuild.fetches.iter().any(|f| {
            f.package == draft.spec.package
                && f.url == draft.spec.url
                && f.output == draft.spec.output
        }) {
            cfg.prebuild.fetches.push(draft.spec.clone());
        }
    }
    cfg.prebuild.allow_hosts.sort();
    cfg.prebuild.allow_hosts.dedup();
}

/// Validate a discovered/suggested output path candidate.
pub fn validate_output_candidate_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("empty output path".into());
    }
    if path.contains("**") || path.contains('?') || path.contains('[') {
        return Err("ambiguous glob rejected".into());
    }
    let star_count = path.matches('*').count();
    if star_count > 1 {
        return Err("multiple wildcards rejected".into());
    }
    if star_count == 1 {
        let Some((dir, file)) = path.rsplit_once('/') else {
            return Err("top-level glob rejected".into());
        };
        if dir.is_empty() || file != "*.node" {
            return Err("only final-component *.node globs are considered candidates".into());
        }
        // Globs may be discovered but are not sealable as declared_outputs yet.
    }
    if path.ends_with('/') {
        return Err("directory outputs rejected; declare files".into());
    }
    let normalized = path.replace('\\', "/");
    if normalized.contains("node_modules/") || normalized.starts_with(".bin/") {
        return Err("outputs under node_modules/ or .bin/ rejected".into());
    }
    if normalized.split('/').count() > 8 {
        return Err("output path too deep".into());
    }
    // Reuse seal path rules for non-glob paths.
    if !normalized.contains('*') {
        validate_declared_output(&normalized).map_err(|e| e.to_string())?;
    } else {
        // Validate parent components manually.
        let parent = normalized.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
        for comp in Path::new(parent).components() {
            match comp {
                Component::Normal(s) => {
                    let s = s.to_string_lossy();
                    if s == ".." || s == "." {
                        return Err("invalid path component".into());
                    }
                }
                Component::CurDir => {}
                _ => return Err("path traversal rejected".into()),
            }
        }
    }
    Ok(())
}

fn push_candidate(out: &mut Vec<OutputCandidate>, path: String, source: OutputCandidateSource) {
    let path = path.replace('\\', "/");
    if out.iter().any(|c| c.path == path) {
        return;
    }
    match validate_output_candidate_path(&path) {
        Ok(()) => out.push(OutputCandidate {
            path,
            source,
            safe: true,
            reject_reason: None,
        }),
        Err(reason) => out.push(OutputCandidate {
            path,
            source,
            safe: false,
            reject_reason: Some(reason),
        }),
    }
}

fn read_package_meta(package_dir: &Path) -> weave_core::Result<PackageMeta> {
    let path = package_dir.join("package.json");
    let text = fs::read_to_string(&path).map_err(|source| weave_core::Error::Io {
        path: path.clone(),
        source,
    })?;
    let v: Value = serde_json::from_str(&text).map_err(|err| weave_core::Error::InvalidState {
        path,
        reason: format!("invalid package.json: {err}"),
    })?;
    let name = v.get("name").and_then(|x| x.as_str()).map(str::to_owned);
    let mut scripts = BTreeMap::new();
    if let Some(obj) = v.get("scripts").and_then(|x| x.as_object()) {
        for (k, val) in obj {
            if let Some(s) = val.as_str() {
                scripts.insert(k.clone(), s.to_owned());
            }
        }
    }
    let binary = v.get("binary").map(|b| BinaryField {
        module_name: b
            .get("module_name")
            .and_then(|x| x.as_str())
            .map(str::to_owned),
        module_path: b
            .get("module_path")
            .and_then(|x| x.as_str())
            .map(str::to_owned),
    });
    let mut weave_declared_outputs = Vec::new();
    if let Some(arr) = v
        .pointer("/weave/declaredOutputs")
        .and_then(|x| x.as_array())
    {
        for item in arr {
            if let Some(s) = item.as_str() {
                weave_declared_outputs.push(s.to_owned());
            }
        }
    }
    let gypfile = v.get("gypfile").and_then(|x| x.as_bool()).unwrap_or(false);
    Ok(PackageMeta {
        name,
        scripts,
        binary,
        weave_declared_outputs,
        gypfile,
    })
}

fn parse_gyp_target_name(text: &str) -> Option<String> {
    // binding.gyp is JSON-like (comments allowed). Pull first "target_name": "…".
    let mut search = text;
    while let Some(idx) = search.find("\"target_name\"") {
        let after = search[idx + "\"target_name\"".len()..].trim_start();
        let after = after.strip_prefix(':')?.trim_start();
        let after = after.strip_prefix('"')?;
        let end = after.find('"')?;
        let name = &after[..end];
        if !name.is_empty() && !name.contains('/') && !name.contains("..") {
            return Some(name.to_owned());
        }
        search = &search[idx + 1..];
    }
    None
}

fn script_body_unsafe(body: &str) -> (bool, Option<String>) {
    let lower = body.to_ascii_lowercase();
    let patterns = [
        ("curl ", "network download via curl"),
        ("wget ", "network download via wget"),
        ("| sh", "pipe to shell"),
        ("|sh", "pipe to shell"),
        ("| bash", "pipe to shell"),
        ("powershell", "powershell invocation"),
        ("invoke-webrequest", "network download"),
        ("rm -rf /", "destructive root delete"),
        ("../", "parent-directory reference in script"),
    ];
    for (pat, reason) in patterns {
        if lower.contains(pat) {
            return (true, Some(reason.into()));
        }
    }
    (false, None)
}

fn static_write_hints(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    // Very conservative: writeFileSync("rel") / writeFileSync('rel')
    for (prefix, quote) in [
        ("writeFileSync(\"", '"'),
        ("writeFileSync('", '\''),
        ("writeFile(\"", '"'),
        ("writeFile('", '\''),
        ("outputFileSync(\"", '"'),
        ("outputFileSync('", '\''),
    ] {
        let mut rest = src;
        while let Some(idx) = rest.find(prefix) {
            let after = &rest[idx + prefix.len()..];
            if let Some(end) = after.find(quote) {
                let path = &after[..end];
                if is_plausible_static_rel_path(path) {
                    out.push(path.replace('\\', "/"));
                }
                rest = &after[end + 1..];
            } else {
                break;
            }
        }
    }
    out
}

fn is_plausible_static_rel_path(path: &str) -> bool {
    if path.is_empty() || path.contains('$') || path.contains('`') || path.contains('+') {
        return false;
    }
    // Allow `..` through so validate_output_candidate_path can record a rejection.
    if path.starts_with('/') && !path.contains("..") {
        return false;
    }
    path.contains('.') || path.contains('/') || path.contains("..")
}

fn classify_discovery(
    name_l: &str,
    scripts: &[DiscoveredScript],
    meta: &PackageMeta,
    unsafe_script: bool,
) -> ExecNeedClass {
    if unsafe_script
        || name_l.contains("electron-chromedriver")
        || name_l.ends_with("-installer-script")
    {
        return ExecNeedClass::UnsupportedUnsafe;
    }
    let native = meta.gypfile
        || name_l.contains("sqlite3")
        || name_l.contains("bcrypt")
        || name_l.contains("fsevents")
        || name_l.contains("sharp")
        || name_l.contains("node-sass")
        || name_l.ends_with("-native");
    if native {
        return ExecNeedClass::NativeBuild;
    }
    if !scripts.is_empty() {
        return ExecNeedClass::GeneratedFiles;
    }
    ExecNeedClass::ExtractionOnly
}

fn discovery_reason(
    class: ExecNeedClass,
    scripts: &[DiscoveredScript],
    candidates: &[OutputCandidate],
    unsafe_script: bool,
) -> String {
    if unsafe_script {
        return "static analysis flagged install script as unsafe — never auto-approved".into();
    }
    match class {
        ExecNeedClass::UnsupportedUnsafe => {
            "classified unsupported/unsafe — never auto-executed".into()
        }
        ExecNeedClass::NativeBuild => {
            let scripts: Vec<_> = scripts.iter().map(|s| s.name.as_str()).collect();
            let safe = candidates.iter().filter(|c| c.safe).count();
            format!("native/gyp package; scripts={scripts:?}; safe_output_candidates={safe}")
        }
        ExecNeedClass::GeneratedFiles => {
            let scripts: Vec<_> = scripts.iter().map(|s| s.name.as_str()).collect();
            let safe = candidates.iter().filter(|c| c.safe).count();
            format!(
                "lifecycle scripts {scripts:?}; safe_output_candidates={safe} (candidates ≠ allowed)"
            )
        }
        ExecNeedClass::ExtractionOnly => "extraction-only".into(),
        ExecNeedClass::RuntimeInstall => "runtime install mutation".into(),
    }
}

fn render_suggestion_toml(
    allow_packages: &[String],
    declared_outputs: &BTreeMap<String, Vec<String>>,
) -> String {
    let mut out = String::from(
        "# Suggested by `weave exec suggest` — REVIEW BEFORE USE.\n\
         # Discovery never enables execution. Keep enabled = false until intentional.\n\
         # Plain `weave switch` stays execution-free; use --with-exec only after review.\n\
         [execution]\n\
         enabled = false\n\
         profile = \"offline\"\n",
    );
    out.push_str("allow_packages = [");
    for (i, p) in allow_packages.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&format!("{p:?}"));
    }
    out.push_str("]\n");
    out.push_str("allow_scripts = [\"install\"]\n\n");
    out.push_str("[execution.declared_outputs]\n");
    for (pkg, paths) in declared_outputs {
        out.push_str(&format!("{pkg} = ["));
        for (i, p) in paths.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!("{p:?}"));
        }
        out.push_str("]\n");
    }
    out
}

/// Resolve a package directory for discovery given project root + package name/key.
pub fn resolve_package_dir_for_discovery(
    project_root: &Path,
    package_key: &str,
    package_name: Option<&str>,
) -> Option<PathBuf> {
    let key_path = if package_key.starts_with("node_modules/") {
        project_root.join(".weave/candidate").join(package_key)
    } else if let Some(name) = package_name {
        project_root
            .join(".weave/candidate/node_modules")
            .join(name)
    } else {
        project_root.join(".weave/candidate").join(package_key)
    };
    if key_path.join("package.json").is_file() {
        return Some(key_path);
    }
    let live = if package_key.starts_with("node_modules/") {
        project_root.join(package_key)
    } else if let Some(name) = package_name {
        project_root.join("node_modules").join(name)
    } else {
        project_root.join("node_modules").join(package_key)
    };
    if live.join("package.json").is_file() {
        return Some(live);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/policy-discovery")
            .join(name)
    }

    #[test]
    fn discovers_native_binding_without_executing() {
        let d = discover_package_dir(&fixture("native-binding")).unwrap();
        assert!(d.metadata_loaded);
        assert_eq!(d.class, ExecNeedClass::NativeBuild);
        assert!(d.needs_execution);
        assert!(d.discovered_scripts.iter().any(|s| s.name == "install"));
        let node = d
            .output_candidates
            .iter()
            .find(|c| c.path == "build/Release/demo_native.node")
            .expect("gyp target candidate");
        assert!(node.safe);
        assert_eq!(node.source, OutputCandidateSource::BindingGyp);
    }

    #[test]
    fn discovers_esbuild_like_bin_hint() {
        let d = discover_package_dir(&fixture("esbuild-like")).unwrap();
        assert_eq!(d.class, ExecNeedClass::GeneratedFiles);
        assert!(d
            .output_candidates
            .iter()
            .any(|c| c.path == "bin/esbuild" && c.safe));
    }

    #[test]
    fn rejects_unsafe_script_and_paths() {
        let d = discover_package_dir(&fixture("unsafe-curl")).unwrap();
        assert_eq!(d.class, ExecNeedClass::UnsupportedUnsafe);
        assert!(!d.needs_execution);
        assert!(d.discovered_scripts.iter().any(|s| s.unsafe_body));
        // Parent escape from install.js static scan must be rejected as a candidate.
        assert!(
            d.output_candidates
                .iter()
                .any(|c| !c.safe && c.path.contains("..")),
            "{:?}",
            d.output_candidates
        );
    }

    #[test]
    fn suggestion_never_enables_and_skips_unsafe() {
        let native = discover_package_dir(&fixture("native-binding")).unwrap();
        let unsafe_pkg = discover_package_dir(&fixture("unsafe-curl")).unwrap();
        let cfg = ExecutionConfig::default();
        let suggestion = suggest_execution_policy(&[native, unsafe_pkg], &cfg);
        assert!(!suggestion.enabled_suggestion);
        assert!(suggestion.allow_packages.iter().any(|p| p == "demo-native"));
        assert!(!suggestion
            .allow_packages
            .iter()
            .any(|p| p == "unsafe-curl-pkg"));
        assert!(suggestion
            .blocked_packages
            .iter()
            .any(|b| b.name == "unsafe-curl-pkg"));
        assert!(suggestion.toml_fragment.contains("enabled = false"));
        assert!(!suggestion.toml_fragment.contains("enabled = true"));
    }

    #[test]
    fn reject_ambiguous_output_paths() {
        assert!(validate_output_candidate_path("**/*.node").is_err());
        assert!(validate_output_candidate_path("../x.node").is_err());
        assert!(validate_output_candidate_path("/abs/x.node").is_err());
        assert!(validate_output_candidate_path("build/").is_err());
        assert!(validate_output_candidate_path("*.node").is_err());
        assert!(validate_output_candidate_path("build/Release/*.node").is_ok());
        assert!(validate_output_candidate_path("generated/hello.txt").is_ok());
    }

    #[test]
    fn weave_hint_outputs_are_candidates_not_approvals() {
        let d = discover_package_dir(&fixture("weave-hint")).unwrap();
        assert!(d
            .output_candidates
            .iter()
            .any(|c| c.path == "generated/out.txt"
                && c.source == OutputCandidateSource::PackageWeaveHint));
        let cfg = ExecutionConfig::default();
        assert_eq!(d.review_against(&cfg), PolicyReviewStatus::NeedsReview);
    }

    #[test]
    fn merge_suggestion_preserves_enabled_false() {
        let native = discover_package_dir(&fixture("native-binding")).unwrap();
        let mut cfg = ExecutionConfig::default();
        assert!(!cfg.enabled);
        let suggestion = suggest_execution_policy(&[native], &cfg);
        merge_suggestion_into_config(&mut cfg, &suggestion);
        assert!(!cfg.enabled);
        assert!(cfg.package_allowed("demo-native"));
        assert!(!cfg.outputs_for("demo-native").is_empty());
    }

    #[test]
    fn prebuild_suggestion_merge_never_enables_or_opens_profile() {
        let host = weave_core::HostPlatform::current();
        let dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/prebuild-resolve/author-sri");
        let report = crate::prebuild_resolve::resolve_native_prebuilds_at(
            &dir,
            &ExecutionConfig::default(),
            &host,
            "137",
        )
        .unwrap();
        let mut cfg = ExecutionConfig::default();
        assert_eq!(cfg.profile, "offline");
        let suggestion = suggest_execution_policy_with_prebuilds(&[], &[report], &cfg);
        assert!(!suggestion.prebuild_fetches.is_empty());
        assert!(!suggestion.enabled_suggestion);
        merge_suggestion_into_config(&mut cfg, &suggestion);
        assert!(!cfg.enabled);
        assert_eq!(cfg.profile, "offline");
        assert!(!cfg.prebuild.fetches.is_empty());
        assert!(cfg
            .prebuild
            .allow_hosts
            .iter()
            .any(|h| h == "cdn.example.com"));
        assert!(!suggestion.toml_fragment.contains("enabled = true"));
    }
}
