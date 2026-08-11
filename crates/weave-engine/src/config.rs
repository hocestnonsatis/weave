//! Project-local Weave configuration.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Current on-disk config schema version.
pub const WEAVE_CONFIG_VERSION: u32 = 1;

/// Contents of `.weave/config.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Schema version.
    #[serde(default)]
    pub version: u32,
    /// Absolute or tilde-expanded path hint for the global store (informational).
    pub store_path: String,
    /// Materialization format version used when environments are created later.
    pub materialization_version: String,
    /// Opt-in sandboxed execution (ADR-0018). Defaults to disabled.
    #[serde(default)]
    pub execution: ExecutionConfig,
}

/// Sandboxed lifecycle/native execution policy (ADR-0018).
///
/// Environment variables alone must never enable execution. Only this
/// version-controlled config (plus an explicit CLI invocation) can.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionConfig {
    /// Master switch. Default `false`.
    #[serde(default)]
    pub enabled: bool,
    /// Sandbox network profile: `"offline"` (default) or `"prebuild-fetch"`.
    #[serde(default = "default_exec_profile")]
    pub profile: String,
    /// Package names allowed to run. Empty means none (even when enabled).
    #[serde(default)]
    pub allow_packages: Vec<String>,
    /// Lifecycle script names allowed (e.g. `install`).
    #[serde(default = "default_allow_scripts")]
    pub allow_scripts: Vec<String>,
    /// Declared relative output paths per package name that may be sealed to CAS.
    #[serde(default)]
    pub declared_outputs: BTreeMap<String, Vec<String>>,
    /// Allow weaker sandbox if bwrap is missing (always refuse).
    #[serde(default)]
    pub allow_weak_sandbox: bool,
    /// Allowlisted prebuild fetch policy (Phase 10). Ignored when profile is offline.
    #[serde(default)]
    pub prebuild: PrebuildConfig,
}

/// Narrowly scoped allowlisted prebuild download policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PrebuildConfig {
    /// Exact hostnames permitted for HTTPS fetches (no wildcards).
    #[serde(default)]
    pub allow_hosts: Vec<String>,
    /// Explicit per-package fetch specifications (no URL discovery).
    #[serde(default)]
    pub fetches: Vec<PrebuildFetchSpec>,
}

/// One explicitly configured prebuild artifact fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrebuildFetchSpec {
    /// Package name (must also appear in `allow_packages` to run).
    pub package: String,
    /// Absolute HTTPS URL to the prebuild artifact.
    pub url: String,
    /// Required SRI (`sha256-…` / `sha512-…`). Missing integrity fails closed.
    pub integrity: String,
    /// Relative output path under the package root (must be in declared_outputs).
    pub output: String,
    /// When set, host Node ABI must match or the fetch is refused.
    #[serde(default)]
    pub node_abi: Option<String>,
    /// When set, host npm OS token must match.
    #[serde(default)]
    pub os: Option<String>,
    /// When set, host npm CPU token must match.
    #[serde(default)]
    pub cpu: Option<String>,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            profile: default_exec_profile(),
            allow_packages: Vec::new(),
            allow_scripts: default_allow_scripts(),
            declared_outputs: BTreeMap::new(),
            allow_weak_sandbox: false,
            prebuild: PrebuildConfig::default(),
        }
    }
}

impl ExecutionConfig {
    /// True only when the version-controlled config enables execution.
    ///
    /// Deliberately ignores `WEAVE_EXEC`, `WEAVE_EXEC_TESTS`, and similar env
    /// vars so ambient CI leakage cannot turn execution on.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Whether the configured profile permits allowlisted network fetches.
    pub fn allows_prebuild_network(&self) -> bool {
        self.profile == "prebuild-fetch"
    }

    /// Validate execution policy constraints.
    pub fn validate(&self) -> Result<(), String> {
        match self.profile.as_str() {
            "offline" | "prebuild-fetch" => {}
            "open" => {
                return Err(
                    "execution.profile \"open\" is rejected (ADR-0018); use offline or prebuild-fetch"
                        .into(),
                );
            }
            other => {
                return Err(format!(
                    "execution.profile {other:?} is not supported; use \"offline\" or \"prebuild-fetch\""
                ));
            }
        }
        if self.allow_weak_sandbox {
            return Err(
                "execution.allow_weak_sandbox is not supported (fail closed on bwrap)".into(),
            );
        }
        for name in &self.allow_scripts {
            if !matches!(
                name.as_str(),
                "preinstall" | "install" | "postinstall" | "prepare"
            ) {
                return Err(format!(
                    "execution.allow_scripts contains unsupported script {name:?}"
                ));
            }
        }
        self.prebuild.validate(self)?;
        Ok(())
    }

    /// Whether `package_name` is allowlisted (and allow list is non-empty).
    pub fn package_allowed(&self, package_name: &str) -> bool {
        self.allow_packages.iter().any(|p| p == package_name)
    }

    /// Declared seal paths for a package, if any.
    pub fn outputs_for(&self, package_name: &str) -> &[String] {
        self.declared_outputs
            .get(package_name)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Fetch specs configured for `package_name`.
    pub fn prebuild_fetches_for(&self, package_name: &str) -> Vec<&PrebuildFetchSpec> {
        self.prebuild
            .fetches
            .iter()
            .filter(|f| f.package == package_name)
            .collect()
    }
}

impl PrebuildConfig {
    fn validate(&self, exec: &ExecutionConfig) -> Result<(), String> {
        if self.fetches.is_empty() {
            return Ok(());
        }
        if exec.profile == "offline" {
            // Specs may exist for planning, but network is disabled — OK to keep
            // them declared; runtime refuses. Still validate structural safety.
        }
        for host in &self.allow_hosts {
            if host.is_empty() || host.contains('/') || host.contains(':') || host.contains('*') {
                return Err(format!(
                    "execution.prebuild.allow_hosts entry {host:?} must be an exact hostname"
                ));
            }
        }
        for fetch in &self.fetches {
            if fetch.package.is_empty() {
                return Err("execution.prebuild.fetches[].package must be non-empty".into());
            }
            if fetch.integrity.trim().is_empty() {
                return Err(format!(
                    "execution.prebuild.fetches for {:?} missing required integrity (fail closed)",
                    fetch.package
                ));
            }
            if let Err(err) = weave_core::Integrity::parse(&fetch.integrity) {
                return Err(format!(
                    "execution.prebuild.fetches for {:?} has invalid integrity: {err}",
                    fetch.package
                ));
            }
            let (scheme, host) = parse_https_host(&fetch.url)
                .map_err(|e| format!("execution.prebuild.fetches for {:?}: {e}", fetch.package))?;
            if scheme != "https" {
                return Err(format!(
                    "execution.prebuild.fetches for {:?}: only https URLs are allowed",
                    fetch.package
                ));
            }
            if host.is_empty() {
                return Err(format!(
                    "execution.prebuild.fetches for {:?}: URL missing host",
                    fetch.package
                ));
            }
            if !self.allow_hosts.iter().any(|h| h == &host) {
                return Err(format!(
                    "execution.prebuild.fetches for {:?}: host {host:?} is not in allow_hosts",
                    fetch.package
                ));
            }
            if !exec
                .outputs_for(&fetch.package)
                .iter()
                .any(|o| o == &fetch.output)
            {
                return Err(format!(
                    "execution.prebuild.fetches for {:?}: output {:?} is not in declared_outputs",
                    fetch.package, fetch.output
                ));
            }
        }
        Ok(())
    }
}

/// Parse `https://host/...` → (scheme, host). Fail closed on anything else.
pub fn parse_https_host(raw: &str) -> Result<(String, String), String> {
    let raw = raw.trim();
    let Some((scheme, rest)) = raw.split_once("://") else {
        return Err("URL must include a scheme".into());
    };
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "https" && scheme != "http" {
        return Err(format!("unsupported URL scheme {scheme:?}"));
    }
    let host_port = rest.split('/').next().unwrap_or("");
    let host = host_port
        .split('@')
        .next_back()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if host.is_empty() || host.contains(' ') {
        return Err("URL missing host".into());
    }
    Ok((scheme, host))
}

fn default_exec_profile() -> String {
    "offline".into()
}

fn default_allow_scripts() -> Vec<String> {
    vec!["install".into()]
}

impl ProjectConfig {
    /// Build a default config pointing at `store_path`.
    pub fn new(store_path: impl Into<String>) -> Self {
        Self {
            version: WEAVE_CONFIG_VERSION,
            store_path: store_path.into(),
            materialization_version: weave_fs_version().to_owned(),
            execution: ExecutionConfig::default(),
        }
    }

    /// Load `.weave/config.toml` from a project root.
    pub fn load(project_root: &std::path::Path) -> weave_core::Result<Self> {
        Ok(Self::load_compat(project_root)?.0)
    }

    /// Load config with compatibility warnings (Phase 13).
    ///
    /// - Missing `version` defaults via serde to 0 → treated as version 1 after migrate.
    /// - Future versions fail closed with a clear migration message.
    /// - Execution validation still fails closed.
    pub fn load_compat(project_root: &std::path::Path) -> weave_core::Result<(Self, Vec<String>)> {
        use weave_core::{Error, WEAVE_CONFIG, WEAVE_DIR};
        let path = project_root.join(WEAVE_DIR).join(WEAVE_CONFIG);
        let text = std::fs::read_to_string(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        let mut cfg: Self = toml::from_str(&text).map_err(|err| Error::InvalidState {
            path: path.clone(),
            reason: format!(
                "invalid config.toml: {err}. See docs/adoption.md — fix or re-run weave init \
                 after backing up .weave/config.toml"
            ),
        })?;
        let mut warnings = Vec::new();
        if cfg.version == 0 {
            warnings.push(
                "config version missing/0 — treating as Weave config v1 (write-back recommended)"
                    .into(),
            );
            cfg.version = WEAVE_CONFIG_VERSION;
        } else if cfg.version > WEAVE_CONFIG_VERSION {
            return Err(Error::InvalidState {
                path: path.clone(),
                reason: format!(
                    "config version {} is newer than this Weave binary supports \
                     (max {WEAVE_CONFIG_VERSION}). Upgrade Weave or restore an older config.",
                    cfg.version
                ),
            });
        } else if cfg.version < WEAVE_CONFIG_VERSION {
            warnings.push(format!(
                "config version {} is older than current ({WEAVE_CONFIG_VERSION}); \
                 fields use defaults for missing keys",
                cfg.version
            ));
            cfg.version = WEAVE_CONFIG_VERSION;
        }
        if let Err(reason) = cfg.execution.validate() {
            return Err(Error::InvalidState {
                path,
                reason: format!(
                    "{reason}. Fix [execution] in .weave/config.toml — Weave fails closed \
                     (docs/adoption.md)."
                ),
            });
        }
        Ok((cfg, warnings))
    }

    /// Serialize to TOML for atomic writes.
    pub fn to_toml_string(&self) -> weave_core::Result<String> {
        toml::to_string_pretty(self).map_err(|err| weave_core::Error::InvalidState {
            path: std::path::PathBuf::from("config.toml"),
            reason: format!("serialize config: {err}"),
        })
    }
}

fn weave_fs_version() -> &'static str {
    weave_fs::materialization_version()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_defaults_disabled() {
        let cfg = ProjectConfig::new("/tmp/store");
        assert!(!cfg.execution.is_enabled());
        assert_eq!(cfg.execution.profile, "offline");
        assert!(cfg.execution.allow_packages.is_empty());
        assert!(!cfg.execution.allows_prebuild_network());
    }

    #[test]
    fn env_vars_do_not_affect_is_enabled() {
        std::env::set_var("WEAVE_EXEC", "1");
        std::env::set_var("WEAVE_EXEC_TESTS", "1");
        let cfg = ExecutionConfig::default();
        assert!(!cfg.is_enabled());
        std::env::remove_var("WEAVE_EXEC");
        std::env::remove_var("WEAVE_EXEC_TESTS");
    }

    #[test]
    fn rejects_open_profile() {
        let cfg = ExecutionConfig {
            profile: "open".into(),
            ..ExecutionConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn accepts_prebuild_fetch_profile_with_allowlist() {
        let mut declared = BTreeMap::new();
        declared.insert("demo".into(), vec!["prebuilds/x.node".into()]);
        let sri = weave_core::Integrity::sha256_sri(b"fixture-bytes").raw;
        let cfg = ExecutionConfig {
            profile: "prebuild-fetch".into(),
            allow_packages: vec!["demo".into()],
            declared_outputs: declared,
            prebuild: PrebuildConfig {
                allow_hosts: vec!["cdn.example.com".into()],
                fetches: vec![PrebuildFetchSpec {
                    package: "demo".into(),
                    url: "https://cdn.example.com/x.node".into(),
                    integrity: sri,
                    output: "prebuilds/x.node".into(),
                    node_abi: Some("137".into()),
                    os: Some("linux".into()),
                    cpu: Some("x64".into()),
                }],
            },
            ..ExecutionConfig::default()
        };
        assert!(cfg.validate().is_ok(), "{:?}", cfg.validate());
        assert!(cfg.allows_prebuild_network());
    }

    #[test]
    fn rejects_fetch_host_not_allowlisted() {
        let mut declared = BTreeMap::new();
        declared.insert("demo".into(), vec!["out.node".into()]);
        let sri = weave_core::Integrity::sha256_sri(b"x").raw;
        let cfg = ExecutionConfig {
            profile: "prebuild-fetch".into(),
            declared_outputs: declared,
            prebuild: PrebuildConfig {
                allow_hosts: vec!["cdn.example.com".into()],
                fetches: vec![PrebuildFetchSpec {
                    package: "demo".into(),
                    url: "https://evil.example/x.node".into(),
                    integrity: sri,
                    output: "out.node".into(),
                    node_abi: None,
                    os: None,
                    cpu: None,
                }],
            },
            ..ExecutionConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_http_url() {
        let mut declared = BTreeMap::new();
        declared.insert("demo".into(), vec!["out.node".into()]);
        let sri = weave_core::Integrity::sha256_sri(b"x").raw;
        let cfg = ExecutionConfig {
            profile: "prebuild-fetch".into(),
            declared_outputs: declared,
            prebuild: PrebuildConfig {
                allow_hosts: vec!["cdn.example.com".into()],
                fetches: vec![PrebuildFetchSpec {
                    package: "demo".into(),
                    url: "http://cdn.example.com/x.node".into(),
                    integrity: sri,
                    output: "out.node".into(),
                    node_abi: None,
                    os: None,
                    cpu: None,
                }],
            },
            ..ExecutionConfig::default()
        };
        assert!(cfg.validate().unwrap_err().contains("https"));
    }

    #[test]
    fn rejects_future_config_version() {
        let tmp = tempfile::tempdir().unwrap();
        let weave = tmp.path().join(".weave");
        std::fs::create_dir_all(&weave).unwrap();
        std::fs::write(
            weave.join("config.toml"),
            "version = 99\nstore_path = \"/tmp\"\nmaterialization_version = \"1\"\n",
        )
        .unwrap();
        let err = ProjectConfig::load(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("newer"));
    }

    #[test]
    fn migrates_missing_version_with_warning() {
        let tmp = tempfile::tempdir().unwrap();
        let weave = tmp.path().join(".weave");
        std::fs::create_dir_all(&weave).unwrap();
        // version omitted — serde default 0
        std::fs::write(
            weave.join("config.toml"),
            "store_path = \"/tmp/store\"\nmaterialization_version = \"test\"\n",
        )
        .unwrap();
        let (cfg, warnings) = ProjectConfig::load_compat(tmp.path()).unwrap();
        assert_eq!(cfg.version, WEAVE_CONFIG_VERSION);
        assert!(!warnings.is_empty());
        assert!(!cfg.execution.enabled);
    }
}
