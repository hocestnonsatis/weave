//! Allowlisted HTTPS prebuild fetch (ADR-0018 Phase 10).
//!
//! Narrow opt-in path: only explicitly configured package/URL/integrity tuples
//! may be downloaded, and only when `execution.profile = "prebuild-fetch"` plus
//! the dual gate (`enabled` + `--with-exec`). Plain `weave switch` never
//! reaches this module. Offline remains the default profile.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use weave_core::{Error, HostPlatform, Integrity};
use weave_store::{hash_bytes, ArtifactId, ContentStore};

use crate::config::{parse_https_host, ExecutionConfig, PrebuildFetchSpec, ProjectConfig};
use crate::exec::{
    apply_sealed_outputs, build_exec_identity, persist_exec_cache, probe_node_identity,
    refuse_live_node_modules, seal_declared_outputs, validate_declared_output, ExecCacheRecord,
    ExecSealReport,
};

/// Maximum redirects followed while re-validating each hop's host.
const MAX_REDIRECTS: usize = 5;

/// Transport abstraction so tests can simulate redirects / bodies without network.
pub trait PrebuildTransport: Send + Sync {
    /// Perform one HTTPS GET without automatically following redirects.
    fn get_no_redirect(&self, url: &str) -> weave_core::Result<PrebuildHttpResponse>;
}

/// Minimal HTTP response used by the prebuild fetcher.
#[derive(Debug, Clone)]
pub struct PrebuildHttpResponse {
    /// Status code.
    pub status: u16,
    /// Optional Location header (redirects).
    pub location: Option<String>,
    /// Response body (empty for redirects).
    pub body: Vec<u8>,
}

/// Default ureq-backed transport (redirects disabled; we follow manually).
#[derive(Debug, Default)]
pub struct UreqPrebuildTransport {
    user_agent: String,
}

impl UreqPrebuildTransport {
    /// Create with Weave user-agent.
    pub fn new() -> Self {
        Self {
            user_agent: format!("weave/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

impl PrebuildTransport for UreqPrebuildTransport {
    fn get_no_redirect(&self, url: &str) -> weave_core::Result<PrebuildHttpResponse> {
        // Manual redirect following: disable ureq redirects and treat all HTTP
        // statuses as Ok so 3xx Location headers stay inspectable.
        let config = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .user_agent(self.user_agent.as_str())
            .build();
        let agent = ureq::Agent::new_with_config(config);
        let resp = agent.get(url).call().map_err(|err| Error::FetchFailed {
            url: url.to_owned(),
            reason: err.to_string(),
        })?;
        let status = resp.status().as_u16();
        let location = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let mut body = Vec::new();
        resp.into_body()
            .as_reader()
            .read_to_end(&mut body)
            .map_err(|err| Error::FetchFailed {
                url: url.to_owned(),
                reason: err.to_string(),
            })?;
        Ok(PrebuildHttpResponse {
            status,
            location,
            body,
        })
    }
}

/// Dry-run / planning view of one prebuild fetch (never contacts the network).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrebuildPlanEntry {
    /// Package name.
    pub package: String,
    /// Configured URL.
    pub url: String,
    /// Host extracted from URL.
    pub host: String,
    /// Whether host is on allow_hosts.
    pub host_allowed: bool,
    /// Configured SRI.
    pub integrity: String,
    /// Declared relative output path.
    pub output: String,
    /// Whether host OS matches the optional constraint.
    pub os_match: bool,
    /// Whether host CPU matches the optional constraint.
    pub cpu_match: bool,
    /// Whether Node ABI matches the optional constraint.
    pub abi_match: bool,
    /// Whether this fetch is selected for the current host.
    pub selected: bool,
    /// Whether performing it would require network (profile permitting).
    pub needs_network: bool,
    /// Human-readable status.
    pub reason: String,
}

/// Provenance record persisted with a verified prebuild artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrebuildProvenance {
    /// Package name.
    pub package: String,
    /// Final URL after approved redirects.
    pub url: String,
    /// SRI string.
    pub integrity: String,
    /// Relative output path.
    pub output: String,
    /// npm OS token.
    pub os: String,
    /// npm CPU token.
    pub cpu: String,
    /// Node ABI.
    pub node_abi: String,
    /// Node version.
    pub node_version: String,
    /// Content-addressed id of the verified blob.
    pub artifact_id: String,
    /// Cache key for this prebuild identity.
    pub cache_key: String,
    /// Profile used (`prebuild-fetch`).
    pub profile: String,
}

/// Report from ensuring prebuild outputs on a candidate package directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrebuildEnsureReport {
    /// Seal report after placing declared outputs.
    pub seal: ExecSealReport,
    /// True when verified bytes came from cache (no network).
    pub cache_hit: bool,
    /// Provenance for the fetched/cached artifact.
    pub provenance: PrebuildProvenance,
}

/// Build dry-run prebuild plan entries for a package (no network).
pub fn plan_prebuild_for_package(
    cfg: &ExecutionConfig,
    package: &str,
) -> weave_core::Result<Vec<PrebuildPlanEntry>> {
    let host = HostPlatform::current();
    let (abi, _ver) =
        probe_node_identity().unwrap_or_else(|_| ("unknown".into(), "unknown".into()));
    let mut out = Vec::new();
    for spec in cfg.prebuild_fetches_for(package) {
        out.push(plan_one(cfg, spec, &host, &abi)?);
    }
    Ok(out)
}

/// Build dry-run plan for all configured prebuild fetches.
pub fn plan_all_prebuilds(cfg: &ExecutionConfig) -> weave_core::Result<Vec<PrebuildPlanEntry>> {
    let host = HostPlatform::current();
    let (abi, _ver) =
        probe_node_identity().unwrap_or_else(|_| ("unknown".into(), "unknown".into()));
    let mut out = Vec::new();
    for spec in &cfg.prebuild.fetches {
        out.push(plan_one(cfg, spec, &host, &abi)?);
    }
    out.sort_by(|a, b| (&a.package, &a.url).cmp(&(&b.package, &b.url)));
    Ok(out)
}

fn plan_one(
    cfg: &ExecutionConfig,
    spec: &PrebuildFetchSpec,
    host: &HostPlatform,
    abi: &str,
) -> weave_core::Result<PrebuildPlanEntry> {
    let (scheme, hostname) = parse_https_host(&spec.url).map_err(|reason| Error::InvalidState {
        path: PathBuf::from(".weave/config.toml"),
        reason,
    })?;
    let host_allowed = scheme == "https"
        && cfg
            .prebuild
            .allow_hosts
            .iter()
            .any(|h| h.eq_ignore_ascii_case(&hostname));
    let os_match = spec.os.as_ref().map(|o| o == host.npm_os()).unwrap_or(true);
    let cpu_match = spec
        .cpu
        .as_ref()
        .map(|c| c == host.npm_cpu())
        .unwrap_or(true);
    let abi_match = spec.node_abi.as_ref().map(|a| a == abi).unwrap_or(true);
    let selected = os_match && cpu_match && abi_match;
    let needs_network = selected && cfg.allows_prebuild_network();
    let reason = if scheme != "https" {
        "rejected: non-HTTPS URL".into()
    } else if !host_allowed {
        format!("host {hostname:?} not in allow_hosts")
    } else if !os_match {
        format!("OS mismatch: need {:?} have {}", spec.os, host.npm_os())
    } else if !cpu_match {
        format!("CPU mismatch: need {:?} have {}", spec.cpu, host.npm_cpu())
    } else if !abi_match {
        format!("Node ABI mismatch: need {:?} have {abi}", spec.node_abi)
    } else if !cfg.allows_prebuild_network() {
        "selected but profile=offline (no network); dry-run only".into()
    } else {
        "selected for allowlisted HTTPS prebuild fetch".into()
    };
    Ok(PrebuildPlanEntry {
        package: spec.package.clone(),
        url: spec.url.clone(),
        host: hostname,
        host_allowed,
        integrity: spec.integrity.clone(),
        output: spec.output.clone(),
        os_match,
        cpu_match,
        abi_match,
        selected,
        needs_network,
        reason,
    })
}

/// Select the single matching fetch spec for the current host, if any.
pub fn select_prebuild_spec<'a>(
    cfg: &'a ExecutionConfig,
    package: &str,
) -> weave_core::Result<Option<&'a PrebuildFetchSpec>> {
    let host = HostPlatform::current();
    let (abi, _) = probe_node_identity()?;
    let mut matches = Vec::new();
    for spec in cfg.prebuild_fetches_for(package) {
        let plan = plan_one(cfg, spec, &host, &abi)?;
        if plan.selected {
            matches.push(spec);
        }
    }
    match matches.len() {
        0 => Ok(None),
        1 => Ok(Some(matches[0])),
        _ => Err(Error::InvalidState {
            path: PathBuf::from(".weave/config.toml"),
            reason: format!(
                "multiple prebuild fetches match package {package:?} for this platform/ABI; \
                 narrow os/cpu/node_abi constraints"
            ),
        }),
    }
}

/// Download (or reuse cache) a verified prebuild into `package_dir` and seal declared outputs.
///
/// `dry_run` validates selection/policy and returns without network or writes.
pub fn ensure_prebuild_on_candidate<T: PrebuildTransport + ?Sized>(
    project_root: &Path,
    package: &str,
    package_dir: &Path,
    transport: &T,
    dry_run: bool,
) -> weave_core::Result<Option<PrebuildEnsureReport>> {
    let cfg = ProjectConfig::load(project_root)?;
    if cfg.execution.prebuild_fetches_for(package).is_empty() {
        return Ok(None);
    }

    let Some(spec) = select_prebuild_spec(&cfg.execution, package)? else {
        // Specs exist but none match platform/ABI — fail closed when profile wants fetch.
        if cfg.execution.allows_prebuild_network() {
            return Err(Error::InvalidState {
                path: PathBuf::from(".weave/config.toml"),
                reason: format!(
                    "prebuild fetch configured for {package:?} but no spec matches \
                     current OS/CPU/Node ABI (fail closed)"
                ),
            });
        }
        return Ok(None);
    };

    let host = HostPlatform::current();
    let (abi, node_version) = probe_node_identity()?;
    let plan = plan_one(&cfg.execution, spec, &host, &abi)?;
    if !plan.host_allowed || !plan.selected {
        return Err(Error::InvalidState {
            path: PathBuf::from(".weave/config.toml"),
            reason: format!("prebuild fetch refused: {}", plan.reason),
        });
    }
    if !cfg.execution.allows_prebuild_network() {
        return Err(Error::InvalidState {
            path: PathBuf::from(".weave/config.toml"),
            reason: format!(
                "prebuild fetch for {package:?} requires execution.profile = \"prebuild-fetch\" \
                 (offline profile refuses network)"
            ),
        });
    }

    validate_declared_output(&spec.output)?;
    let declared = cfg.execution.outputs_for(package).to_vec();
    if !declared.iter().any(|o| o == &spec.output) {
        return Err(Error::InvalidState {
            path: PathBuf::from(".weave/config.toml"),
            reason: format!(
                "prebuild output {:?} is not in declared_outputs for {package:?}",
                spec.output
            ),
        });
    }

    refuse_live_node_modules(project_root, package_dir)?;

    if dry_run {
        // Planning already done; no network / no writes.
        return Ok(None);
    }

    let integrity = Integrity::parse(&spec.integrity)?;
    let cache_key = prebuild_cache_key(spec, host.npm_os(), host.npm_cpu(), &abi);
    let store = ContentStore::open(PathBuf::from(&cfg.store_path))?;

    let host_id = HostIdentity {
        os: host.npm_os(),
        cpu: host.npm_cpu(),
        abi: &abi,
        node_version: &node_version,
    };
    let (bytes, cache_hit, final_url) = if let Some((id, prov)) = lookup_prebuild_cache(&cache_key)?
    {
        if !store.contains(&id) {
            fetch_and_store(
                transport,
                &cfg.execution,
                spec,
                &integrity,
                &store,
                &cache_key,
                host_id,
            )?
        } else {
            let bytes = store.get(&id)?;
            integrity.verify(&bytes, package)?;
            // Re-check recorded provenance ABI/platform.
            if prov.node_abi != abi || prov.os != host.npm_os() || prov.cpu != host.npm_cpu() {
                return Err(Error::InvalidState {
                    path: PathBuf::from("exec/prebuild-cache"),
                    reason: "prebuild cache provenance ABI/platform mismatch (fail closed)".into(),
                });
            }
            (bytes, true, prov.url)
        }
    } else {
        fetch_and_store(
            transport,
            &cfg.execution,
            spec,
            &integrity,
            &store,
            &cache_key,
            host_id,
        )?
    };

    // Write only the declared output path into the isolated package dir.
    let dest = package_dir.join(&spec.output);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(&dest, &bytes).map_err(|source| Error::Io {
        path: dest.clone(),
        source,
    })?;

    // Seal only declared outputs (must all exist — Phase 10 expects fetch to satisfy them).
    let mut seal = seal_declared_outputs(&store, package_dir, &declared)?;
    let identity = build_exec_identity(&cfg.execution, package, package_dir)?;
    seal.cache_key = identity.cache_key();
    let _ = persist_exec_cache(&ExecCacheRecord {
        package: package.to_owned(),
        output_artifact_id: seal.output_artifact_id.to_string(),
        cache_key: seal.cache_key.clone(),
        sealed_paths: seal.sealed_paths.clone(),
        node_abi: abi.clone(),
        os: host.npm_os().to_owned(),
        cpu: host.npm_cpu().to_owned(),
        profile: cfg.execution.profile.clone(),
    });

    // Ensure apply path is consistent (idempotent copy from sealed CAS).
    apply_sealed_outputs(&store, &seal.output_artifact_id, package_dir, &declared)?;

    let provenance = PrebuildProvenance {
        package: package.to_owned(),
        url: final_url,
        integrity: spec.integrity.clone(),
        output: spec.output.clone(),
        os: host.npm_os().to_owned(),
        cpu: host.npm_cpu().to_owned(),
        node_abi: abi,
        node_version,
        artifact_id: hash_bytes(&bytes).to_string(),
        cache_key,
        profile: "prebuild-fetch".into(),
    };
    Ok(Some(PrebuildEnsureReport {
        seal,
        cache_hit,
        provenance,
    }))
}

struct HostIdentity<'a> {
    os: &'a str,
    cpu: &'a str,
    abi: &'a str,
    node_version: &'a str,
}

fn fetch_and_store<T: PrebuildTransport + ?Sized>(
    transport: &T,
    cfg: &ExecutionConfig,
    spec: &PrebuildFetchSpec,
    integrity: &Integrity,
    store: &ContentStore,
    cache_key: &str,
    host: HostIdentity<'_>,
) -> weave_core::Result<(Vec<u8>, bool, String)> {
    let (bytes, final_url) =
        https_fetch_allowlisted(transport, &spec.url, &cfg.prebuild.allow_hosts)?;
    integrity
        .verify(&bytes, &spec.package)
        .map_err(|err| Error::InvalidState {
            path: PathBuf::from(&spec.url),
            reason: format!("prebuild integrity verification failed: {err}"),
        })?;
    let id = hash_bytes(&bytes);
    store.put(&bytes, Some(&id))?;
    let provenance = PrebuildProvenance {
        package: spec.package.clone(),
        url: final_url.clone(),
        integrity: spec.integrity.clone(),
        output: spec.output.clone(),
        os: host.os.to_owned(),
        cpu: host.cpu.to_owned(),
        node_abi: host.abi.to_owned(),
        node_version: host.node_version.to_owned(),
        artifact_id: id.to_string(),
        cache_key: cache_key.to_owned(),
        profile: "prebuild-fetch".into(),
    };
    write_prebuild_cache(cache_key, &id, &provenance)?;
    Ok((bytes, false, final_url))
}

/// Fetch `url` over HTTPS, following redirects only to allowlisted hosts.
pub fn https_fetch_allowlisted<T: PrebuildTransport + ?Sized>(
    transport: &T,
    url: &str,
    allow_hosts: &[String],
) -> weave_core::Result<(Vec<u8>, String)> {
    let mut current = url.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        validate_fetch_url(&current, allow_hosts)?;
        let resp = transport.get_no_redirect(&current)?;
        if (300..400).contains(&resp.status) {
            let loc = resp.location.ok_or_else(|| Error::FetchFailed {
                url: current.clone(),
                reason: format!("redirect {} without Location", resp.status),
            })?;
            let next = resolve_redirect(&current, &loc)?;
            // Fail closed if redirect target host is not allowlisted.
            validate_fetch_url(&next, allow_hosts).map_err(|err| Error::FetchFailed {
                url: next.clone(),
                reason: format!("redirect to denied host/url: {err}"),
            })?;
            current = next;
            continue;
        }
        if resp.status != 200 {
            return Err(Error::FetchFailed {
                url: current,
                reason: format!("unexpected HTTP status {}", resp.status),
            });
        }
        return Ok((resp.body, current));
    }
    Err(Error::FetchFailed {
        url: url.to_owned(),
        reason: format!("too many redirects (>{MAX_REDIRECTS})"),
    })
}

/// Validate a URL for allowlisted HTTPS fetch.
pub fn validate_fetch_url(url: &str, allow_hosts: &[String]) -> weave_core::Result<()> {
    let (scheme, host) = parse_https_host(url).map_err(|reason| Error::InvalidState {
        path: PathBuf::from(url),
        reason,
    })?;
    if scheme != "https" {
        return Err(Error::InvalidState {
            path: PathBuf::from(url),
            reason: "only https URLs are allowed for prebuild fetch".into(),
        });
    }
    if !allow_hosts.iter().any(|h| h.eq_ignore_ascii_case(&host)) {
        return Err(Error::InvalidState {
            path: PathBuf::from(url),
            reason: format!("host {host:?} is not in execution.prebuild.allow_hosts"),
        });
    }
    Ok(())
}

fn resolve_redirect(base: &str, location: &str) -> weave_core::Result<String> {
    let loc = location.trim();
    if loc.starts_with("https://") || loc.starts_with("http://") {
        return Ok(loc.to_owned());
    }
    // Relative redirect — resolve against base.
    let (scheme, host) = parse_https_host(base).map_err(|reason| Error::InvalidState {
        path: PathBuf::from(base),
        reason,
    })?;
    if loc.starts_with('/') {
        return Ok(format!("{scheme}://{host}{loc}"));
    }
    Err(Error::FetchFailed {
        url: base.to_owned(),
        reason: format!("unsupported relative redirect {loc:?}"),
    })
}

fn prebuild_cache_key(spec: &PrebuildFetchSpec, os: &str, cpu: &str, abi: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"weave-prebuild-v1\0");
    for part in [
        spec.package.as_str(),
        spec.url.as_str(),
        spec.integrity.as_str(),
        spec.output.as_str(),
        os,
        cpu,
        abi,
    ] {
        h.update(part.as_bytes());
        h.update(b"\0");
    }
    hex(&h.finalize())
}

fn prebuild_cache_dir() -> weave_core::Result<PathBuf> {
    Ok(weave_store::default_weave_home()?
        .join("exec")
        .join("prebuild-cache"))
}

fn lookup_prebuild_cache(
    cache_key: &str,
) -> weave_core::Result<Option<(ArtifactId, PrebuildProvenance)>> {
    let path = prebuild_cache_dir()?.join(format!("{cache_key}.json"));
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).map_err(|source| Error::Io {
        path: path.clone(),
        source,
    })?;
    let prov: PrebuildProvenance =
        serde_json::from_str(&text).map_err(|err| Error::InvalidState {
            path,
            reason: format!("invalid prebuild cache index: {err}"),
        })?;
    if prov.cache_key != cache_key {
        return Err(Error::InvalidState {
            path: PathBuf::from("exec/prebuild-cache"),
            reason: "prebuild cache_key mismatch".into(),
        });
    }
    let id = ArtifactId::parse(&prov.artifact_id)?;
    Ok(Some((id, prov)))
}

fn write_prebuild_cache(
    cache_key: &str,
    id: &ArtifactId,
    provenance: &PrebuildProvenance,
) -> weave_core::Result<()> {
    let dir = prebuild_cache_dir()?;
    fs::create_dir_all(&dir).map_err(|source| Error::Io {
        path: dir.clone(),
        source,
    })?;
    let path = dir.join(format!("{cache_key}.json"));
    let mut prov = provenance.clone();
    prov.artifact_id = id.to_string();
    prov.cache_key = cache_key.to_owned();
    let bytes = serde_json::to_vec_pretty(&prov).map_err(|err| Error::InvalidState {
        path: path.clone(),
        reason: format!("serialize prebuild cache: {err}"),
    })?;
    fs::write(&path, bytes).map_err(|source| Error::Io { path, source })?;
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

/// In-memory scripted transport for tests.
#[derive(Debug, Default)]
pub struct MockPrebuildTransport {
    /// Map from URL → response. Missing URLs fail.
    pub responses: BTreeMap<String, PrebuildHttpResponse>,
    /// Call count per URL.
    pub hits: std::sync::Mutex<BTreeMap<String, usize>>,
}

impl MockPrebuildTransport {
    /// Create empty mock.
    pub fn new() -> Self {
        Self::default()
    }

    /// Script a response.
    pub fn with(mut self, url: impl Into<String>, resp: PrebuildHttpResponse) -> Self {
        self.responses.insert(url.into(), resp);
        self
    }

    /// How many times `url` was requested.
    pub fn hit_count(&self, url: &str) -> usize {
        self.hits
            .lock()
            .map(|m| m.get(url).copied().unwrap_or(0))
            .unwrap_or(0)
    }
}

impl PrebuildTransport for MockPrebuildTransport {
    fn get_no_redirect(&self, url: &str) -> weave_core::Result<PrebuildHttpResponse> {
        if let Ok(mut hits) = self.hits.lock() {
            *hits.entry(url.to_owned()).or_insert(0) += 1;
        }
        self.responses
            .get(url)
            .cloned()
            .ok_or_else(|| Error::FetchFailed {
                url: url.to_owned(),
                reason: "mock transport: URL not scripted".into(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{PrebuildConfig, ProjectConfig};
    use crate::test_util::lock_weave_home;
    use std::collections::BTreeMap;

    fn sri(bytes: &[u8]) -> String {
        format!("sha256-{}", {
            const T: &[u8; 64] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let d = Sha256::digest(bytes);
            let mut out = String::new();
            for chunk in d.chunks(3) {
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
        })
    }

    fn base_cfg(bytes: &[u8]) -> ExecutionConfig {
        let mut declared = BTreeMap::new();
        declared.insert("demo-prebuild".into(), vec!["prebuilds/addon.node".into()]);
        ExecutionConfig {
            enabled: true,
            profile: "prebuild-fetch".into(),
            allow_packages: vec!["demo-prebuild".into()],
            declared_outputs: declared,
            prebuild: PrebuildConfig {
                allow_hosts: vec!["cdn.example.com".into()],
                fetches: vec![PrebuildFetchSpec {
                    package: "demo-prebuild".into(),
                    url: "https://cdn.example.com/addon.node".into(),
                    integrity: sri(bytes),
                    output: "prebuilds/addon.node".into(),
                    node_abi: None,
                    os: None,
                    cpu: None,
                }],
            },
            ..ExecutionConfig::default()
        }
    }

    #[test]
    fn denies_unallowlisted_host() {
        let err = validate_fetch_url("https://evil.example/x.node", &["cdn.example.com".into()])
            .unwrap_err();
        assert!(err.to_string().contains("not in"));
    }

    #[test]
    fn denies_http_scheme() {
        let err = validate_fetch_url("http://cdn.example.com/x.node", &["cdn.example.com".into()])
            .unwrap_err();
        assert!(err.to_string().contains("https"));
    }

    #[test]
    fn redirect_to_denied_host_fails() {
        let transport = MockPrebuildTransport::new().with(
            "https://cdn.example.com/start",
            PrebuildHttpResponse {
                status: 302,
                location: Some("https://evil.example/x.node".into()),
                body: Vec::new(),
            },
        );
        let err = https_fetch_allowlisted(
            &transport,
            "https://cdn.example.com/start",
            &["cdn.example.com".into()],
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("denied") || err.to_string().contains("not in"),
            "{err}"
        );
    }

    #[test]
    fn allowed_fetch_verifies_integrity() {
        let _g = lock_weave_home();
        let body = b"native-prebuild-bytes";
        let transport = MockPrebuildTransport::new().with(
            "https://cdn.example.com/addon.node",
            PrebuildHttpResponse {
                status: 200,
                location: None,
                body: body.to_vec(),
            },
        );
        let (got, url) = https_fetch_allowlisted(
            &transport,
            "https://cdn.example.com/addon.node",
            &["cdn.example.com".into()],
        )
        .unwrap();
        assert_eq!(got, body);
        assert_eq!(url, "https://cdn.example.com/addon.node");
        Integrity::parse(&sri(body))
            .unwrap()
            .verify(&got, "demo")
            .unwrap();
    }

    #[test]
    fn integrity_mismatch_fails() {
        let body = b"actual";
        let integrity = Integrity::parse(&sri(b"expected")).unwrap();
        assert!(integrity.verify(body, "demo").is_err());
    }

    #[test]
    fn dry_run_plan_needs_network_flag() {
        let cfg = base_cfg(b"x");
        let plan = plan_all_prebuilds(&cfg).unwrap();
        assert_eq!(plan.len(), 1);
        assert!(plan[0].needs_network);
        assert!(plan[0].host_allowed);
        let offline = ExecutionConfig {
            profile: "offline".into(),
            ..cfg
        };
        // Offline profile: fetches may still be listed for planning but needs_network=false.
        // validate() allows fetches under offline for planning — check plan flag.
        let plan = plan_all_prebuilds(&offline).unwrap();
        assert!(!plan[0].needs_network);
        assert!(plan[0].reason.contains("offline"));
    }

    #[test]
    fn cache_hit_skips_second_fetch() {
        let _g = lock_weave_home();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
        let body = b"cached-prebuild-v1";
        let project = tmp.path().join("project");
        fs::create_dir_all(project.join(".weave")).unwrap();
        let store_path = tmp.path().join("store");
        fs::create_dir_all(&store_path).unwrap();

        let mut cfg = ProjectConfig::new(store_path.display().to_string());
        cfg.execution = base_cfg(body);
        fs::write(
            project.join(".weave/config.toml"),
            toml::to_string_pretty(&cfg).unwrap(),
        )
        .unwrap();

        let pkg = tmp.path().join("candidate/node_modules/demo-prebuild");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(
            pkg.join("package.json"),
            r#"{"name":"demo-prebuild","version":"1.0.0"}"#,
        )
        .unwrap();

        let transport = MockPrebuildTransport::new().with(
            "https://cdn.example.com/addon.node",
            PrebuildHttpResponse {
                status: 200,
                location: None,
                body: body.to_vec(),
            },
        );

        let first =
            ensure_prebuild_on_candidate(&project, "demo-prebuild", &pkg, &transport, false)
                .unwrap()
                .expect("report");
        assert!(!first.cache_hit);
        assert_eq!(transport.hit_count("https://cdn.example.com/addon.node"), 1);
        assert_eq!(fs::read(pkg.join("prebuilds/addon.node")).unwrap(), body);

        let second =
            ensure_prebuild_on_candidate(&project, "demo-prebuild", &pkg, &transport, false)
                .unwrap()
                .expect("report");
        assert!(second.cache_hit);
        assert_eq!(
            transport.hit_count("https://cdn.example.com/addon.node"),
            1,
            "must not refetch on cache hit"
        );

        std::env::remove_var("WEAVE_HOME");
    }

    #[test]
    fn abi_mismatch_refuses_fetch() {
        let _g = lock_weave_home();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
        let body = b"abi-bytes";
        let project = tmp.path().join("project");
        fs::create_dir_all(project.join(".weave")).unwrap();
        let store_path = tmp.path().join("store");
        fs::create_dir_all(&store_path).unwrap();

        let mut cfg = ProjectConfig::new(store_path.display().to_string());
        cfg.execution = base_cfg(body);
        cfg.execution.prebuild.fetches[0].node_abi = Some("0".into());
        fs::write(
            project.join(".weave/config.toml"),
            toml::to_string_pretty(&cfg).unwrap(),
        )
        .unwrap();

        let pkg = tmp.path().join("candidate/node_modules/demo-prebuild");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("package.json"), r#"{"name":"demo-prebuild"}"#).unwrap();

        let transport = MockPrebuildTransport::new().with(
            "https://cdn.example.com/addon.node",
            PrebuildHttpResponse {
                status: 200,
                location: None,
                body: body.to_vec(),
            },
        );
        let err = ensure_prebuild_on_candidate(&project, "demo-prebuild", &pkg, &transport, false)
            .unwrap_err();
        assert!(
            err.to_string().contains("ABI") || err.to_string().contains("match"),
            "{err}"
        );
        assert_eq!(transport.hit_count("https://cdn.example.com/addon.node"), 0);
        std::env::remove_var("WEAVE_HOME");
    }

    #[test]
    fn offline_profile_refuses_network() {
        let _g = lock_weave_home();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
        let body = b"offline-bytes";
        let project = tmp.path().join("project");
        fs::create_dir_all(project.join(".weave")).unwrap();
        let store_path = tmp.path().join("store");
        fs::create_dir_all(&store_path).unwrap();

        let mut cfg = ProjectConfig::new(store_path.display().to_string());
        cfg.execution = base_cfg(body);
        cfg.execution.profile = "offline".into();
        fs::write(
            project.join(".weave/config.toml"),
            toml::to_string_pretty(&cfg).unwrap(),
        )
        .unwrap();

        let pkg = tmp.path().join("candidate/node_modules/demo-prebuild");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("package.json"), r#"{"name":"demo-prebuild"}"#).unwrap();

        let transport = MockPrebuildTransport::new().with(
            "https://cdn.example.com/addon.node",
            PrebuildHttpResponse {
                status: 200,
                location: None,
                body: body.to_vec(),
            },
        );
        let err = ensure_prebuild_on_candidate(&project, "demo-prebuild", &pkg, &transport, false)
            .unwrap_err();
        assert!(
            err.to_string().contains("offline") || err.to_string().contains("prebuild-fetch"),
            "{err}"
        );
        assert_eq!(transport.hit_count("https://cdn.example.com/addon.node"), 0);
        std::env::remove_var("WEAVE_HOME");
    }
}
