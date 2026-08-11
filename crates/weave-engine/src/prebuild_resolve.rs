//! Native prebuild resolution from package metadata (ADR-0018 Phase 11).
//!
//! Statically detects how real packages obtain native binaries during install
//! (node-pre-gyp `binary` fields, prebuild-install layouts, concrete HTTPS
//! literals, optional author `weave.prebuildFetches`) **without executing
//! scripts** and **without granting them network access**.
//!
//! Discovered URLs are never auto-approved. Only fully concrete HTTPS URLs with
//! a known SRI may enter a *reviewable* suggestion; everything else is
//! diagnosed for manual policy.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use weave_core::HostPlatform;

use crate::config::{parse_https_host, ExecutionConfig, PrebuildFetchSpec};
use crate::exec::probe_node_identity;
use crate::exec_discover::validate_output_candidate_path;

/// How a native prebuild source pattern was detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrebuildPatternKind {
    /// package.json `binary` (node-pre-gyp / @mapbox/node-pre-gyp).
    NodePreGypBinary,
    /// Conventional `prebuilds/{platform}-{arch}/` layout + prebuild-install.
    PrebuildInstallLayout,
    /// Concrete `https://…` string literal in an install script file.
    InstallScriptHttpsLiteral,
    /// Author-declared `weave.prebuildFetches` in package.json.
    PackageWeavePrebuild,
    /// Well-known package name heuristic (still not an approval).
    KnownPackagePattern,
}

/// Whether Weave can materialize an explicit fetch entry from static metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrebuildResolveStatus {
    /// Already present in project `execution.prebuild.fetches`.
    Configured,
    /// Concrete HTTPS URL + output + integrity — ready for human review/suggest.
    Suggestable,
    /// URL/output known but integrity missing — cannot seal safely yet.
    NeedsIntegrity,
    /// Template tokens remain after substituting known platform values.
    UnresolvedTokens,
    /// Pattern detected but URL/output cannot be established safely.
    Opaque,
    /// HTTP / unsafe host / path — never suggest.
    BlockedUnsafe,
}

/// One detected native artifact requirement (not an approval).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativePrebuildRequirement {
    /// Package name.
    pub package: String,
    /// Detection pattern.
    pub pattern: PrebuildPatternKind,
    /// Resolution status.
    pub status: PrebuildResolveStatus,
    /// Candidate HTTPS URL when concrete (or partially substituted template).
    pub url: Option<String>,
    /// Host extracted from URL when present.
    pub host: Option<String>,
    /// Relative output path under the package root when known.
    pub output: Option<String>,
    /// SRI when statically known (author metadata only).
    pub integrity: Option<String>,
    /// Optional Node ABI constraint inferred for this artifact.
    pub node_abi: Option<String>,
    /// Optional OS constraint.
    pub os: Option<String>,
    /// Optional CPU constraint.
    pub cpu: Option<String>,
    /// Template tokens that blocked full resolution.
    pub unresolved_tokens: Vec<String>,
    /// Why this artifact cannot (yet) be resolved / must be reviewed.
    pub reason: String,
}

/// Per-package native prebuild resolution report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativePrebuildReport {
    /// Package name.
    pub package: String,
    /// Package version when known.
    pub version: Option<String>,
    /// Detected requirements.
    pub requirements: Vec<NativePrebuildRequirement>,
    /// True when any requirement still blocks completeness without policy.
    pub needs_manual_policy: bool,
}

/// Reviewable prebuild fetch draft (only when Suggestable).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestedPrebuildFetch {
    /// Draft fetch spec for human review.
    pub spec: PrebuildFetchSpec,
    /// Host to add to allow_hosts if missing.
    pub host: String,
    /// Detection pattern that produced this draft.
    pub pattern: PrebuildPatternKind,
    /// Reminder text.
    pub note: String,
}

/// Resolve native prebuild requirements for an on-disk package directory.
///
/// Never executes scripts and never contacts the network.
pub fn resolve_native_prebuilds(
    package_dir: &Path,
    cfg: &ExecutionConfig,
) -> weave_core::Result<NativePrebuildReport> {
    let host = HostPlatform::current();
    let (abi, _) = probe_node_identity().unwrap_or_else(|_| ("unknown".into(), "unknown".into()));
    resolve_native_prebuilds_at(package_dir, cfg, &host, &abi)
}

/// Resolve with an explicit platform/ABI (tests / planning).
pub fn resolve_native_prebuilds_at(
    package_dir: &Path,
    cfg: &ExecutionConfig,
    host: &HostPlatform,
    node_abi: &str,
) -> weave_core::Result<NativePrebuildReport> {
    let meta = read_meta(package_dir)?;
    let package = meta.name.clone().unwrap_or_else(|| {
        package_dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".into())
    });
    let mut requirements = Vec::new();

    // 1) Author weave.prebuildFetches (may include integrity → suggestable).
    for raw in &meta.weave_prebuilds {
        requirements.push(from_weave_prebuild(&package, raw, cfg, host, node_abi));
    }

    // 2) node-pre-gyp binary field.
    if let Some(bin) = &meta.binary {
        if let Some(req) = from_node_pre_gyp(&package, &meta, bin, cfg, host, node_abi) {
            requirements.push(req);
        }
    }

    // 3) prebuild-install layout markers.
    if meta.has_prebuild_install_script || package_dir.join("prebuilds").is_dir() {
        if let Some(req) = from_prebuild_install_layout(&package, &meta, cfg, host, node_abi) {
            requirements.push(req);
        }
    }

    // 4) Concrete HTTPS literals in install entrypoints.
    for rel in ["install.js", "scripts/install.js", "binding.gyp"] {
        if rel == "binding.gyp" {
            continue;
        }
        let path = package_dir.join(rel);
        if path.is_file() {
            if let Ok(src) = fs::read_to_string(&path) {
                for url in extract_https_literals(&src) {
                    requirements.push(from_https_literal(
                        &package, &url, cfg, host, node_abi, &meta,
                    ));
                }
            }
        }
    }
    for body in meta.scripts.values() {
        for url in extract_https_literals(body) {
            requirements.push(from_https_literal(
                &package, &url, cfg, host, node_abi, &meta,
            ));
        }
    }

    // 5) Known package heuristics when nothing else fired.
    if requirements.is_empty() {
        if let Some(req) = known_package_heuristic(&package, &meta, cfg, host, node_abi) {
            requirements.push(req);
        }
    }

    // Dedup by (url, output, pattern).
    requirements.sort_by(|a, b| {
        (
            a.pattern as u8,
            a.url.as_deref().unwrap_or(""),
            a.output.as_deref().unwrap_or(""),
        )
            .cmp(&(
                b.pattern as u8,
                b.url.as_deref().unwrap_or(""),
                b.output.as_deref().unwrap_or(""),
            ))
    });
    requirements.dedup_by(|a, b| a.url == b.url && a.output == b.output && a.pattern == b.pattern);

    let needs_manual_policy = requirements.iter().any(|r| {
        !matches!(
            r.status,
            PrebuildResolveStatus::Configured | PrebuildResolveStatus::Suggestable
        )
    }) || (requirements.is_empty()
        && (meta.binary.is_some()
            || meta.has_prebuild_install_script
            || meta.scripts.values().any(|b| {
                let l = b.to_ascii_lowercase();
                l.contains("prebuild") || l.contains("node-pre-gyp") || l.contains("download")
            })));

    // If scripts hint downloads but we found nothing concrete:
    if requirements.is_empty() && needs_manual_policy {
        requirements.push(NativePrebuildRequirement {
            package: package.clone(),
            pattern: PrebuildPatternKind::KnownPackagePattern,
            status: PrebuildResolveStatus::Opaque,
            url: None,
            host: None,
            output: None,
            integrity: None,
            node_abi: Some(node_abi.to_owned()),
            os: Some(host.npm_os().to_owned()),
            cpu: Some(host.npm_cpu().to_owned()),
            unresolved_tokens: Vec::new(),
            reason: "install/native download behavior detected but no concrete HTTPS URL, \
                     output path, and integrity could be established statically — \
                     declare execution.prebuild.fetches manually"
                .into(),
        });
    }

    Ok(NativePrebuildReport {
        package,
        version: meta.version,
        requirements,
        needs_manual_policy,
    })
}

/// Collect suggestable prebuild drafts from a resolution report.
///
/// Never includes entries lacking integrity or using non-HTTPS / denied hosts.
pub fn suggestable_prebuild_fetches(
    report: &NativePrebuildReport,
    cfg: &ExecutionConfig,
) -> Vec<SuggestedPrebuildFetch> {
    let mut out = Vec::new();
    for req in &report.requirements {
        if req.status != PrebuildResolveStatus::Suggestable {
            continue;
        }
        let (Some(url), Some(output), Some(integrity)) =
            (req.url.clone(), req.output.clone(), req.integrity.clone())
        else {
            continue;
        };
        let Ok((_, host)) = parse_https_host(&url) else {
            continue;
        };
        // Skip if already configured identically.
        if cfg.prebuild.fetches.iter().any(|f| {
            f.package == req.package
                && f.url == url
                && f.output == output
                && f.integrity == integrity
        }) {
            continue;
        }
        out.push(SuggestedPrebuildFetch {
            spec: PrebuildFetchSpec {
                package: req.package.clone(),
                url,
                integrity,
                output,
                node_abi: req.node_abi.clone(),
                os: req.os.clone(),
                cpu: req.cpu.clone(),
            },
            host,
            pattern: req.pattern,
            note: "REVIEW ONLY — discovered from package metadata; not auto-approved; \
                   enable profile=prebuild-fetch + dual gate after review"
                .into(),
        });
    }
    out
}

/// Render a TOML fragment for suggested prebuild fetches (review only).
pub fn render_prebuild_suggestion_toml(
    drafts: &[SuggestedPrebuildFetch],
    current_hosts: &[String],
) -> String {
    if drafts.is_empty() {
        return String::new();
    }
    let mut hosts: Vec<String> = current_hosts.to_vec();
    for d in drafts {
        if !hosts.iter().any(|h| h == &d.host) {
            hosts.push(d.host.clone());
        }
    }
    hosts.sort();
    hosts.dedup();

    let mut out = String::from(
        "# Suggested prebuild fetches — REVIEW BEFORE USE.\n\
         # Never auto-approved. Requires execution.profile = \"prebuild-fetch\".\n\
         # Plain switch stays network-free; dual gate = enabled + --with-exec.\n\
         [execution.prebuild]\n\
         allow_hosts = [",
    );
    for (i, h) in hosts.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&format!("{h:?}"));
    }
    out.push_str("]\n\n");
    for d in drafts {
        out.push_str("[[execution.prebuild.fetches]]\n");
        out.push_str(&format!("package = {:?}\n", d.spec.package));
        out.push_str(&format!("url = {:?}\n", d.spec.url));
        out.push_str(&format!("integrity = {:?}\n", d.spec.integrity));
        out.push_str(&format!("output = {:?}\n", d.spec.output));
        if let Some(abi) = &d.spec.node_abi {
            out.push_str(&format!("node_abi = {abi:?}\n"));
        }
        if let Some(os) = &d.spec.os {
            out.push_str(&format!("os = {os:?}\n"));
        }
        if let Some(cpu) = &d.spec.cpu {
            out.push_str(&format!("cpu = {cpu:?}\n"));
        }
        out.push('\n');
    }
    out
}

#[derive(Debug, Clone, Default)]
struct Meta {
    name: Option<String>,
    version: Option<String>,
    scripts: BTreeMap<String, String>,
    binary: Option<BinaryMeta>,
    weave_prebuilds: Vec<WeavePrebuildRaw>,
    has_prebuild_install_script: bool,
    napi_versions: Vec<u32>,
}

#[derive(Debug, Clone, Default)]
struct BinaryMeta {
    module_name: Option<String>,
    module_path: Option<String>,
    remote_path: Option<String>,
    package_name: Option<String>,
    host: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct WeavePrebuildRaw {
    url: String,
    integrity: String,
    output: String,
    node_abi: Option<String>,
    os: Option<String>,
    cpu: Option<String>,
}

fn read_meta(package_dir: &Path) -> weave_core::Result<Meta> {
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
    let version = v.get("version").and_then(|x| x.as_str()).map(str::to_owned);
    let mut scripts = BTreeMap::new();
    if let Some(obj) = v.get("scripts").and_then(|x| x.as_object()) {
        for (k, val) in obj {
            if let Some(s) = val.as_str() {
                scripts.insert(k.clone(), s.to_owned());
            }
        }
    }
    let has_prebuild_install_script = scripts.values().any(|b| {
        let l = b.to_ascii_lowercase();
        l.contains("prebuild-install") || l.contains("prebuild --install")
    });
    let binary = v.get("binary").map(|b| BinaryMeta {
        module_name: b
            .get("module_name")
            .and_then(|x| x.as_str())
            .map(str::to_owned),
        module_path: b
            .get("module_path")
            .and_then(|x| x.as_str())
            .map(str::to_owned),
        remote_path: b
            .get("remote_path")
            .and_then(|x| x.as_str())
            .map(str::to_owned),
        package_name: b
            .get("package_name")
            .and_then(|x| x.as_str())
            .map(str::to_owned),
        host: b.get("host").and_then(|x| x.as_str()).map(str::to_owned),
    });
    let mut weave_prebuilds = Vec::new();
    if let Some(arr) = v
        .pointer("/weave/prebuildFetches")
        .and_then(|x| x.as_array())
    {
        for item in arr {
            let url = item
                .get("url")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_owned();
            let integrity = item
                .get("integrity")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_owned();
            let output = item
                .get("output")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_owned();
            if !url.is_empty() && !output.is_empty() {
                weave_prebuilds.push(WeavePrebuildRaw {
                    url,
                    integrity,
                    output,
                    node_abi: item
                        .get("node_abi")
                        .and_then(|x| x.as_str())
                        .map(str::to_owned),
                    os: item.get("os").and_then(|x| x.as_str()).map(str::to_owned),
                    cpu: item.get("cpu").and_then(|x| x.as_str()).map(str::to_owned),
                });
            }
        }
    }
    let mut napi_versions = Vec::new();
    if let Some(arr) = v
        .pointer("/binary/napi_versions")
        .and_then(|x| x.as_array())
        .or_else(|| v.get("napi_versions").and_then(|x| x.as_array()))
    {
        for item in arr {
            if let Some(n) = item.as_u64() {
                napi_versions.push(n as u32);
            }
        }
    }
    Ok(Meta {
        name,
        version,
        scripts,
        binary,
        weave_prebuilds,
        has_prebuild_install_script,
        napi_versions,
    })
}

fn already_configured(cfg: &ExecutionConfig, package: &str, url: &str, output: &str) -> bool {
    cfg.prebuild
        .fetches
        .iter()
        .any(|f| f.package == package && f.url == url && f.output == output)
}

fn from_weave_prebuild(
    package: &str,
    raw: &WeavePrebuildRaw,
    cfg: &ExecutionConfig,
    host: &HostPlatform,
    node_abi: &str,
) -> NativePrebuildRequirement {
    let mut req = NativePrebuildRequirement {
        package: package.to_owned(),
        pattern: PrebuildPatternKind::PackageWeavePrebuild,
        status: PrebuildResolveStatus::Opaque,
        url: Some(raw.url.clone()),
        host: None,
        output: Some(raw.output.clone()),
        integrity: if raw.integrity.is_empty() {
            None
        } else {
            Some(raw.integrity.clone())
        },
        node_abi: raw.node_abi.clone().or_else(|| Some(node_abi.to_owned())),
        os: raw.os.clone().or_else(|| Some(host.npm_os().to_owned())),
        cpu: raw.cpu.clone().or_else(|| Some(host.npm_cpu().to_owned())),
        unresolved_tokens: Vec::new(),
        reason: String::new(),
    };
    finalize_requirement(&mut req, cfg);
    req
}

fn from_node_pre_gyp(
    package: &str,
    meta: &Meta,
    bin: &BinaryMeta,
    cfg: &ExecutionConfig,
    host: &HostPlatform,
    node_abi: &str,
) -> Option<NativePrebuildRequirement> {
    let host_url = bin.host.as_deref()?;
    let remote = bin.remote_path.as_deref().unwrap_or("");
    let pkg_name = bin
        .package_name
        .as_deref()
        .unwrap_or("{node_abi}-{platform}-{arch}.tar.gz");
    let module_name = bin.module_name.as_deref().unwrap_or(package);
    let module_path = bin.module_path.as_deref().unwrap_or("./lib/binding/");

    let napi = meta.napi_versions.first().copied().unwrap_or(3);
    let ctx = TokenCtx {
        name: package,
        version: meta.version.as_deref().unwrap_or("0.0.0"),
        platform: host.npm_os(),
        arch: host.npm_cpu(),
        node_abi,
        napi_build_version: napi,
        tool: "node-pre-gyp",
        tool_version: "1",
    };
    let (remote_r, mut tokens) = substitute_tokens(remote, &ctx);
    let (pkg_r, tokens2) = substitute_tokens(pkg_name, &ctx);
    tokens.extend(tokens2);
    tokens.sort();
    tokens.dedup();

    let url = join_url(host_url, &format!("{remote_r}{pkg_r}"));
    let (mod_path_r, tokens3) = substitute_tokens(module_path, &ctx);
    tokens.extend(tokens3);
    tokens.sort();
    tokens.dedup();
    let output = format!(
        "{}/{}.node",
        mod_path_r.trim_start_matches("./").trim_end_matches('/'),
        module_name
    );

    let mut req = NativePrebuildRequirement {
        package: package.to_owned(),
        pattern: PrebuildPatternKind::NodePreGypBinary,
        status: PrebuildResolveStatus::Opaque,
        url: Some(url),
        host: None,
        output: Some(output),
        integrity: None,
        node_abi: Some(node_abi.to_owned()),
        os: Some(host.npm_os().to_owned()),
        cpu: Some(host.npm_cpu().to_owned()),
        unresolved_tokens: tokens,
        reason: String::new(),
    };
    finalize_requirement(&mut req, cfg);
    if !req.unresolved_tokens.is_empty()
        && matches!(
            req.status,
            PrebuildResolveStatus::NeedsIntegrity | PrebuildResolveStatus::Suggestable
        )
    {
        req.status = PrebuildResolveStatus::UnresolvedTokens;
        req.reason = format!(
            "node-pre-gyp binary template still has unresolved tokens {:?}; \
             cannot emit a deterministic fetch entry — fill tokens or declare \
             execution.prebuild.fetches manually",
            req.unresolved_tokens
        );
    } else if req.integrity.is_none()
        && matches!(
            req.status,
            PrebuildResolveStatus::NeedsIntegrity | PrebuildResolveStatus::Suggestable
        )
    {
        req.status = PrebuildResolveStatus::NeedsIntegrity;
        req.reason = format!(
            "node-pre-gyp pattern resolved to {} → {:?}, but integrity (SRI) is not \
             present in package metadata — add integrity to execution.prebuild.fetches \
             after verifying the artifact (lifecycle scripts still get no network)",
            req.url.as_deref().unwrap_or("?"),
            req.output
        );
    }
    Some(req)
}

fn from_prebuild_install_layout(
    package: &str,
    meta: &Meta,
    cfg: &ExecutionConfig,
    host: &HostPlatform,
    node_abi: &str,
) -> Option<NativePrebuildRequirement> {
    // Conventional local layout after download — URL itself is usually opaque
    // without scanning install.js. Surface the expected output path.
    let output = format!(
        "prebuilds/{}-{}/node.napi.node",
        host.npm_os(),
        host.npm_cpu()
    );
    let mut req = NativePrebuildRequirement {
        package: package.to_owned(),
        pattern: PrebuildPatternKind::PrebuildInstallLayout,
        status: PrebuildResolveStatus::Opaque,
        url: None,
        host: None,
        output: Some(output.clone()),
        integrity: None,
        node_abi: Some(node_abi.to_owned()),
        os: Some(host.npm_os().to_owned()),
        cpu: Some(host.npm_cpu().to_owned()),
        unresolved_tokens: Vec::new(),
        reason: format!(
            "prebuild-install layout expects output {output:?} for {}/{} abi={node_abi}, \
             but no concrete HTTPS download URL was found in package metadata — \
             declare url+integrity in execution.prebuild.fetches (version={:?})",
            host.npm_os(),
            host.npm_cpu(),
            meta.version
        ),
    };
    if let Some(url) = req.url.as_deref() {
        if already_configured(cfg, package, url, &output) {
            req.status = PrebuildResolveStatus::Configured;
            req.reason = "already configured in execution.prebuild.fetches".into();
        }
    }
    Some(req)
}

fn from_https_literal(
    package: &str,
    url: &str,
    cfg: &ExecutionConfig,
    host: &HostPlatform,
    node_abi: &str,
    meta: &Meta,
) -> NativePrebuildRequirement {
    // Guess output from URL filename when it looks like a native artifact.
    let output = url
        .rsplit('/')
        .next()
        .filter(|f| f.ends_with(".node") || f.ends_with(".tar.gz") || f.ends_with(".tgz"))
        .map(|f| {
            if f.ends_with(".node") {
                format!("prebuilds/{f}")
            } else {
                // Archive — cannot safely know extracted path.
                String::new()
            }
        })
        .filter(|s| !s.is_empty());

    let integrity = meta
        .weave_prebuilds
        .iter()
        .find(|w| w.url == url)
        .map(|w| w.integrity.clone())
        .filter(|s| !s.is_empty());

    let mut req = NativePrebuildRequirement {
        package: package.to_owned(),
        pattern: PrebuildPatternKind::InstallScriptHttpsLiteral,
        status: PrebuildResolveStatus::Opaque,
        url: Some(url.to_owned()),
        host: None,
        output,
        integrity,
        node_abi: Some(node_abi.to_owned()),
        os: Some(host.npm_os().to_owned()),
        cpu: Some(host.npm_cpu().to_owned()),
        unresolved_tokens: Vec::new(),
        reason: String::new(),
    };
    finalize_requirement(&mut req, cfg);
    if req.output.is_none()
        && !matches!(
            req.status,
            PrebuildResolveStatus::BlockedUnsafe | PrebuildResolveStatus::Configured
        )
    {
        req.status = PrebuildResolveStatus::Opaque;
        req.reason = format!(
            "install script references {url}, but the sealed relative output path \
             cannot be determined statically (archive or non-.node URL) — declare \
             output+integrity manually; scripts still receive no network"
        );
    }
    req
}

fn known_package_heuristic(
    package: &str,
    meta: &Meta,
    cfg: &ExecutionConfig,
    host: &HostPlatform,
    node_abi: &str,
) -> Option<NativePrebuildRequirement> {
    let name = package.to_ascii_lowercase();
    if name == "esbuild" || name.starts_with("@esbuild/") {
        let mut req = NativePrebuildRequirement {
            package: package.to_owned(),
            pattern: PrebuildPatternKind::KnownPackagePattern,
            status: PrebuildResolveStatus::Opaque,
            url: None,
            host: None,
            output: Some("bin/esbuild".into()),
            integrity: None,
            node_abi: Some(node_abi.to_owned()),
            os: Some(host.npm_os().to_owned()),
            cpu: Some(host.npm_cpu().to_owned()),
            unresolved_tokens: Vec::new(),
            reason: format!(
                "esbuild-like package typically downloads a platform binary into bin/esbuild \
                 during postinstall (version={:?}); no concrete allowlisted URL+SRI in metadata — \
                 declare execution.prebuild.fetches after reviewing the release artifact",
                meta.version
            ),
        };
        finalize_requirement(&mut req, cfg);
        return Some(req);
    }
    if name == "sharp" || name.ends_with("/sharp") {
        return Some(NativePrebuildRequirement {
            package: package.to_owned(),
            pattern: PrebuildPatternKind::KnownPackagePattern,
            status: PrebuildResolveStatus::Opaque,
            url: None,
            host: Some("github.com".into()),
            output: None,
            integrity: None,
            node_abi: Some(node_abi.to_owned()),
            os: Some(host.npm_os().to_owned()),
            cpu: Some(host.npm_cpu().to_owned()),
            unresolved_tokens: vec!["vendor_url".into(), "libvips_version".into()],
            reason: "sharp downloads libvips/vendor binaries via install; URL selection is \
                     dynamic — Weave will not grant the install script network; declare \
                     explicit prebuild.fetches for the exact vendor tarball if required"
                .into(),
        });
    }
    None
}

fn finalize_requirement(req: &mut NativePrebuildRequirement, cfg: &ExecutionConfig) {
    if let (Some(url), Some(output)) = (req.url.clone(), req.output.clone()) {
        if already_configured(cfg, &req.package, &url, &output) {
            req.status = PrebuildResolveStatus::Configured;
            req.reason = "already configured in execution.prebuild.fetches".into();
            if let Ok((_, host)) = parse_https_host(&url) {
                req.host = Some(host);
            }
            return;
        }
    }

    let Some(url) = req.url.clone() else {
        if req.reason.is_empty() {
            req.status = PrebuildResolveStatus::Opaque;
            req.reason = "no concrete download URL established".into();
        }
        return;
    };

    match parse_https_host(&url) {
        Ok((scheme, host)) => {
            req.host = Some(host);
            if scheme != "https" {
                req.status = PrebuildResolveStatus::BlockedUnsafe;
                req.reason = "non-HTTPS URL rejected for prebuild policy".into();
                return;
            }
            // Host allow_hosts is enforced at fetch time (Phase 10), not during
            // static discovery. Missing hosts remain Suggestable/NeedsIntegrity so
            // `weave exec suggest` can propose adding them — never auto-approved.
        }
        Err(err) => {
            req.status = PrebuildResolveStatus::BlockedUnsafe;
            req.reason = err;
            return;
        }
    }

    if let Some(output) = &req.output {
        if let Err(err) = validate_output_candidate_path(output) {
            req.status = PrebuildResolveStatus::BlockedUnsafe;
            req.reason = format!("unsafe output path: {err}");
            return;
        }
        if output.contains('*') {
            req.status = PrebuildResolveStatus::Opaque;
            req.reason = "output path is a glob — seal requires an exact relative file".into();
            return;
        }
    } else {
        req.status = PrebuildResolveStatus::Opaque;
        req.reason = "output path unknown".into();
        return;
    }

    if !req.unresolved_tokens.is_empty() {
        req.status = PrebuildResolveStatus::UnresolvedTokens;
        req.reason = format!(
            "unresolved template tokens {:?} prevent deterministic URL construction",
            req.unresolved_tokens
        );
        return;
    }

    match &req.integrity {
        Some(sri) if !sri.is_empty() => {
            if weave_core::Integrity::parse(sri).is_err() {
                req.status = PrebuildResolveStatus::BlockedUnsafe;
                req.reason = format!("invalid integrity SRI in metadata: {sri}");
                return;
            }
            req.status = PrebuildResolveStatus::Suggestable;
            req.reason = format!(
                "concrete HTTPS URL + output + integrity established from metadata \
                 ({:?}) — review before adding to execution.prebuild.fetches",
                req.pattern
            );
        }
        _ => {
            req.status = PrebuildResolveStatus::NeedsIntegrity;
            if req.reason.is_empty() {
                req.reason = format!(
                    "URL {:?} and output {:?} known, but integrity is missing — \
                     verify the artifact and set SRI before allowlisting \
                     (never auto-approved; scripts get no network)",
                    req.url, req.output
                );
            }
        }
    }
}

struct TokenCtx<'a> {
    name: &'a str,
    version: &'a str,
    platform: &'a str,
    arch: &'a str,
    node_abi: &'a str,
    napi_build_version: u32,
    tool: &'a str,
    tool_version: &'a str,
}

fn substitute_tokens(input: &str, ctx: &TokenCtx<'_>) -> (String, Vec<String>) {
    let mut out = input.to_owned();
    let replacements = [
        ("{name}", ctx.name),
        ("{version}", ctx.version),
        ("{platform}", ctx.platform),
        ("{arch}", ctx.arch),
        ("{node_abi}", ctx.node_abi),
        ("{configuration}", "Release"),
        ("{tool}", ctx.tool),
        ("{tool_version}", ctx.tool_version),
    ];
    for (token, value) in replacements {
        out = out.replace(token, value);
    }
    let napi = ctx.napi_build_version.to_string();
    out = out.replace("{napi_build_version}", &napi);
    // libc often libc/glibc — leave unresolved if present.
    let mut unresolved = Vec::new();
    for token in [
        "{libc}",
        "{libc_version}",
        "{platform_vendor}",
        "{target_arch}",
        "{module_name}",
    ] {
        if out.contains(token) {
            unresolved.push(token.trim_matches('{').trim_matches('}').to_owned());
        }
    }
    // Any remaining {…} tokens.
    let mut rest = out.as_str();
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        if let Some(end) = after.find('}') {
            let token = &after[..end];
            if !token.is_empty() && !unresolved.iter().any(|t| t == token) {
                unresolved.push(token.to_owned());
            }
            rest = &after[end + 1..];
        } else {
            break;
        }
    }
    (out, unresolved)
}

fn join_url(host: &str, path: &str) -> String {
    let host = host.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    if host.ends_with('/') {
        format!("{host}{path}")
    } else {
        format!("{host}/{path}")
    }
}

fn extract_https_literals(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(idx) = rest.find("https://") {
        let slice = &rest[idx..];
        let end = slice
            .find(|c: char| {
                c.is_whitespace()
                    || c == '"'
                    || c == '\''
                    || c == '`'
                    || c == ')'
                    || c == ';'
                    || c == '<'
                    || c == '>'
            })
            .unwrap_or(slice.len());
        let url = slice[..end].trim_end_matches(['/', '\\', '.', ',']);
        // Skip obvious non-artifact docs links if path has no file-like suffix —
        // still record download-ish URLs.
        if url.len() > "https://".len() + 3 {
            out.push(url.to_owned());
        }
        rest = &slice[end.max(1)..];
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PrebuildConfig;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/prebuild-resolve")
            .join(name)
    }

    fn empty_cfg() -> ExecutionConfig {
        ExecutionConfig::default()
    }

    #[test]
    fn resolves_node_pre_gyp_needs_integrity() {
        let host = HostPlatform::current();
        let report =
            resolve_native_prebuilds_at(&fixture("node-pre-gyp-like"), &empty_cfg(), &host, "137")
                .unwrap();
        assert!(!report.requirements.is_empty());
        let req = &report.requirements[0];
        assert_eq!(req.pattern, PrebuildPatternKind::NodePreGypBinary);
        assert!(
            matches!(
                req.status,
                PrebuildResolveStatus::NeedsIntegrity | PrebuildResolveStatus::UnresolvedTokens
            ),
            "{:?}",
            req
        );
        assert!(req.url.as_ref().unwrap().starts_with("https://"));
        assert!(req.output.as_ref().unwrap().ends_with(".node"));
        assert!(report.needs_manual_policy);
    }

    #[test]
    fn weave_author_prebuild_is_suggestable() {
        let host = HostPlatform::current();
        let report =
            resolve_native_prebuilds_at(&fixture("author-sri"), &empty_cfg(), &host, "137")
                .unwrap();
        let req = report
            .requirements
            .iter()
            .find(|r| r.pattern == PrebuildPatternKind::PackageWeavePrebuild)
            .expect("weave prebuild");
        assert_eq!(req.status, PrebuildResolveStatus::Suggestable);
        let drafts = suggestable_prebuild_fetches(&report, &empty_cfg());
        assert_eq!(drafts.len(), 1);
        assert!(!drafts[0].spec.integrity.is_empty());
        let toml = render_prebuild_suggestion_toml(&drafts, &[]);
        assert!(toml.contains("allow_hosts"));
        assert!(toml.contains("[[execution.prebuild.fetches]]"));
        assert!(!toml.contains("enabled = true"));
    }

    #[test]
    fn never_auto_approves_without_integrity() {
        let host = HostPlatform::current();
        let report =
            resolve_native_prebuilds_at(&fixture("node-pre-gyp-like"), &empty_cfg(), &host, "137")
                .unwrap();
        assert!(suggestable_prebuild_fetches(&report, &empty_cfg()).is_empty());
    }

    #[test]
    fn opaque_prebuild_install_explains_gap() {
        let host = HostPlatform::current();
        let report = resolve_native_prebuilds_at(
            &fixture("prebuild-install-like"),
            &empty_cfg(),
            &host,
            "137",
        )
        .unwrap();
        assert!(report.needs_manual_policy);
        assert!(report.requirements.iter().any(|r| {
            r.pattern == PrebuildPatternKind::PrebuildInstallLayout
                && r.status == PrebuildResolveStatus::Opaque
        }));
    }

    #[test]
    fn https_literal_without_output_is_opaque_or_needs_integrity() {
        let host = HostPlatform::current();
        let report =
            resolve_native_prebuilds_at(&fixture("esbuild-download"), &empty_cfg(), &host, "137")
                .unwrap();
        assert!(report.needs_manual_policy);
        assert!(report.requirements.iter().any(|r| {
            matches!(
                r.pattern,
                PrebuildPatternKind::InstallScriptHttpsLiteral
                    | PrebuildPatternKind::KnownPackagePattern
            )
        }));
    }

    #[test]
    fn configured_fetch_marked_configured() {
        let host = HostPlatform::current();
        let mut declared = BTreeMap::new();
        declared.insert(
            "author-sri-pkg".into(),
            vec!["prebuilds/linux-x64/addon.node".into()],
        );
        let cfg = ExecutionConfig {
            profile: "prebuild-fetch".into(),
            declared_outputs: declared,
            prebuild: PrebuildConfig {
                allow_hosts: vec!["cdn.example.com".into()],
                fetches: vec![PrebuildFetchSpec {
                    package: "author-sri-pkg".into(),
                    url: "https://cdn.example.com/author-sri-pkg/linux-x64-137.node".into(),
                    integrity: "sha256-S2RVnWoXKGSQi+m2nuQsFbkM6x6AqX/GsamehuwdrMs=".into(),
                    output: "prebuilds/linux-x64/addon.node".into(),
                    node_abi: Some("137".into()),
                    os: Some("linux".into()),
                    cpu: Some("x64".into()),
                }],
            },
            ..ExecutionConfig::default()
        };
        assert!(cfg.validate().is_ok(), "{:?}", cfg.validate());
        let report =
            resolve_native_prebuilds_at(&fixture("author-sri"), &cfg, &host, "137").unwrap();
        assert!(report
            .requirements
            .iter()
            .any(|r| r.status == PrebuildResolveStatus::Configured));
    }

    #[test]
    fn http_literal_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("package.json"),
            r#"{
  "name": "bad-http",
  "version": "1.0.0",
  "weave": {
    "prebuildFetches": [{
      "url": "http://cdn.example.com/x.node",
      "integrity": "sha256-S2RVnWoXKGSQi+m2nuQsFbkM6x6AqX/GsamehuwdrMs=",
      "output": "prebuilds/x.node"
    }]
  }
}"#,
        )
        .unwrap();
        let host = HostPlatform::current();
        let report = resolve_native_prebuilds_at(tmp.path(), &empty_cfg(), &host, "137").unwrap();
        assert!(report
            .requirements
            .iter()
            .any(|r| r.status == PrebuildResolveStatus::BlockedUnsafe));
        assert!(suggestable_prebuild_fetches(&report, &empty_cfg()).is_empty());
    }

    #[test]
    fn sharp_like_is_opaque_manual_policy() {
        let host = HostPlatform::current();
        let report =
            resolve_native_prebuilds_at(&fixture("sharp-like"), &empty_cfg(), &host, "137")
                .unwrap();
        assert!(report.needs_manual_policy);
        assert!(report.requirements.iter().any(|r| {
            r.pattern == PrebuildPatternKind::KnownPackagePattern
                && r.status == PrebuildResolveStatus::Opaque
        }));
        assert!(suggestable_prebuild_fetches(&report, &empty_cfg()).is_empty());
    }

    #[test]
    fn suggestion_toml_never_enables_or_opens_network() {
        let host = HostPlatform::current();
        let report =
            resolve_native_prebuilds_at(&fixture("author-sri"), &empty_cfg(), &host, "137")
                .unwrap();
        let drafts = suggestable_prebuild_fetches(&report, &empty_cfg());
        assert!(!drafts.is_empty());
        let toml = render_prebuild_suggestion_toml(&drafts, &[]);
        assert!(!toml.contains("enabled = true"));
        assert!(!toml.contains("profile = \"open\""));
        assert!(toml.contains("REVIEW"));
    }
}
