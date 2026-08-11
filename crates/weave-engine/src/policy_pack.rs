//! Version-controlled prebuild policy packs (Phase 13).
//!
//! Packs are reviewed TOML documents that propose `allow_hosts` +
//! `prebuild.fetches` entries. Applying a pack never enables execution and
//! never sets `profile = "open"`.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::{ExecutionConfig, PrebuildConfig, ProjectConfig};

/// Current policy pack schema version.
pub const POLICY_PACK_VERSION: u32 = 1;

/// On-disk reviewed policy pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyPack {
    /// Schema version.
    pub version: u32,
    /// Stable pack id (for docs / audit).
    pub id: String,
    /// Human description.
    #[serde(default)]
    pub description: String,
    /// Prebuild allowlist fragment.
    #[serde(default)]
    pub prebuild: PrebuildConfig,
    /// Optional packages to add to allow_packages (still not an enablement).
    #[serde(default)]
    pub allow_packages: Vec<String>,
    /// Optional declared outputs to merge.
    #[serde(default)]
    pub declared_outputs: std::collections::BTreeMap<String, Vec<String>>,
}

/// Result of loading / applying a pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyPackApplyReport {
    /// Pack id.
    pub pack_id: String,
    /// Hosts added.
    pub hosts_added: Vec<String>,
    /// Fetches added.
    pub fetches_added: usize,
    /// Packages added to allow_packages.
    pub packages_added: Vec<String>,
    /// Whether execution.enabled was left unchanged (always true by design).
    pub enabled_unchanged: bool,
    /// Profile left unchanged (always true by design).
    pub profile_unchanged: bool,
    /// Reminder text.
    pub note: String,
}

/// Load a policy pack from disk and validate it.
pub fn load_policy_pack(path: &Path) -> weave_core::Result<PolicyPack> {
    let text = fs::read_to_string(path).map_err(|source| weave_core::Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let pack: PolicyPack =
        toml::from_str(&text).map_err(|err| weave_core::Error::InvalidState {
            path: path.to_path_buf(),
            reason: format!("invalid policy pack: {err}"),
        })?;
    validate_policy_pack(&pack).map_err(|reason| weave_core::Error::InvalidState {
        path: path.to_path_buf(),
        reason,
    })?;
    Ok(pack)
}

/// Validate pack constraints (HTTPS, SRI, hosts, schema).
pub fn validate_policy_pack(pack: &PolicyPack) -> Result<(), String> {
    if pack.version == 0 || pack.version > POLICY_PACK_VERSION {
        return Err(format!(
            "unsupported policy pack version {} (supported: 1..={POLICY_PACK_VERSION})",
            pack.version
        ));
    }
    if pack.id.trim().is_empty() {
        return Err("policy pack id must be non-empty".into());
    }
    // Reuse execution prebuild validation with a synthetic offline-disabled exec cfg.
    let mut exec = ExecutionConfig {
        enabled: false,
        profile: "prebuild-fetch".into(),
        allow_packages: pack.allow_packages.clone(),
        declared_outputs: pack.declared_outputs.clone(),
        prebuild: pack.prebuild.clone(),
        ..ExecutionConfig::default()
    };
    // Ensure declared outputs cover fetches for validate().
    for f in &pack.prebuild.fetches {
        exec.declared_outputs
            .entry(f.package.clone())
            .or_default()
            .push(f.output.clone());
    }
    exec.validate()
}

/// Merge a pack into project execution config **without enabling execution**.
pub fn apply_policy_pack(cfg: &mut ProjectConfig, pack: &PolicyPack) -> PolicyPackApplyReport {
    let was_enabled = cfg.execution.enabled;
    let was_profile = cfg.execution.profile.clone();

    let mut hosts_added = Vec::new();
    for h in &pack.prebuild.allow_hosts {
        if !cfg
            .execution
            .prebuild
            .allow_hosts
            .iter()
            .any(|e| e.eq_ignore_ascii_case(h))
        {
            cfg.execution.prebuild.allow_hosts.push(h.clone());
            hosts_added.push(h.clone());
        }
    }
    cfg.execution.prebuild.allow_hosts.sort();
    cfg.execution.prebuild.allow_hosts.dedup();

    let mut fetches_added = 0usize;
    for fetch in &pack.prebuild.fetches {
        if !cfg
            .execution
            .prebuild
            .fetches
            .iter()
            .any(|f| f.package == fetch.package && f.url == fetch.url && f.output == fetch.output)
        {
            cfg.execution.prebuild.fetches.push(fetch.clone());
            fetches_added += 1;
        }
        let outs = cfg
            .execution
            .declared_outputs
            .entry(fetch.package.clone())
            .or_default();
        if !outs.iter().any(|o| o == &fetch.output) {
            outs.push(fetch.output.clone());
        }
    }

    let mut packages_added = Vec::new();
    for pkg in &pack.allow_packages {
        if !cfg.execution.allow_packages.iter().any(|p| p == pkg) {
            cfg.execution.allow_packages.push(pkg.clone());
            packages_added.push(pkg.clone());
        }
    }
    for (pkg, outs) in &pack.declared_outputs {
        let entry = cfg
            .execution
            .declared_outputs
            .entry(pkg.clone())
            .or_default();
        for o in outs {
            if !entry.iter().any(|e| e == o) {
                entry.push(o.clone());
            }
        }
    }
    cfg.execution.allow_packages.sort();
    cfg.execution.allow_packages.dedup();

    // Hard guarantees.
    cfg.execution.enabled = was_enabled;
    cfg.execution.profile = was_profile.clone();

    PolicyPackApplyReport {
        pack_id: pack.id.clone(),
        hosts_added,
        fetches_added,
        packages_added,
        enabled_unchanged: cfg.execution.enabled == was_enabled,
        profile_unchanged: cfg.execution.profile == was_profile,
        note: "Policy pack merged for review. execution.enabled and profile were not changed. \
               Dual gate still required: enable in config + weave switch --with-exec."
            .into(),
    }
}

/// Render a pack as TOML (for docs / examples).
pub fn render_policy_pack_toml(pack: &PolicyPack) -> weave_core::Result<String> {
    toml::to_string_pretty(pack).map_err(|err| weave_core::Error::InvalidState {
        path: std::path::PathBuf::from("policy-pack.toml"),
        reason: format!("serialize pack: {err}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PrebuildFetchSpec;
    use weave_core::Integrity;

    #[test]
    fn apply_never_enables_or_opens() {
        let sri = Integrity::sha256_sri(b"pack-bytes").raw;
        let pack = PolicyPack {
            version: 1,
            id: "demo-pack".into(),
            description: "test".into(),
            prebuild: PrebuildConfig {
                allow_hosts: vec!["cdn.example.com".into()],
                fetches: vec![PrebuildFetchSpec {
                    package: "demo".into(),
                    url: "https://cdn.example.com/x.node".into(),
                    integrity: sri,
                    output: "prebuilds/x.node".into(),
                    node_abi: None,
                    os: None,
                    cpu: None,
                }],
            },
            allow_packages: vec!["demo".into()],
            declared_outputs: Default::default(),
        };
        validate_policy_pack(&pack).unwrap();
        let mut cfg = ProjectConfig::new("/tmp/store");
        assert!(!cfg.execution.enabled);
        assert_eq!(cfg.execution.profile, "offline");
        let report = apply_policy_pack(&mut cfg, &pack);
        assert!(!cfg.execution.enabled);
        assert_eq!(cfg.execution.profile, "offline");
        assert!(report.enabled_unchanged);
        assert!(report.profile_unchanged);
        assert_eq!(report.fetches_added, 1);
        assert!(cfg.execution.package_allowed("demo"));
    }

    #[test]
    fn rejects_http_in_pack() {
        let sri = Integrity::sha256_sri(b"x").raw;
        let pack = PolicyPack {
            version: 1,
            id: "bad".into(),
            description: String::new(),
            prebuild: PrebuildConfig {
                allow_hosts: vec!["cdn.example.com".into()],
                fetches: vec![PrebuildFetchSpec {
                    package: "demo".into(),
                    url: "http://cdn.example.com/x.node".into(),
                    integrity: sri,
                    output: "x.node".into(),
                    node_abi: None,
                    os: None,
                    cpu: None,
                }],
            },
            allow_packages: vec!["demo".into()],
            declared_outputs: Default::default(),
        };
        assert!(validate_policy_pack(&pack).is_err());
    }
}
