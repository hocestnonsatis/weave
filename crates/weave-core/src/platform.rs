//! Host platform identity and npm `os` / `cpu` constraint matching.

use serde::{Deserialize, Serialize};

use crate::PackageNode;

/// Platform slice used for environment identity and package filtering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostPlatform {
    /// Rust/`std` OS name (`linux`, `macos`, `windows`, …).
    pub os: String,
    /// Rust/`std` architecture (`x86_64`, `aarch64`, …).
    pub arch: String,
}

impl HostPlatform {
    /// Capture the current host.
    pub fn current() -> Self {
        Self {
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
        }
    }

    /// npm-style OS token (`darwin`, `linux`, `win32`, …).
    pub fn npm_os(&self) -> &str {
        match self.os.as_str() {
            "macos" => "darwin",
            "windows" => "win32",
            other => other,
        }
    }

    /// npm-style CPU token (`x64`, `arm64`, `ia32`, …).
    pub fn npm_cpu(&self) -> &str {
        match self.arch.as_str() {
            "x86_64" => "x64",
            "aarch64" => "arm64",
            "x86" => "ia32",
            other => other,
        }
    }
}

/// Result of evaluating a package's `os` / `cpu` constraints against a host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformFit {
    /// Constraints empty or host matches.
    Compatible,
    /// Host does not match, but the package is optional — skip quietly.
    SkipOptional,
    /// Host does not match and the package is required — hard error.
    RejectRequired,
}

/// Evaluate npm `os` / `cpu` fields for `node` against `host`.
///
/// Rules mirror npm:
/// - empty list → unrestricted
/// - positive entries → allow-list (any match wins)
/// - `!name` entries → deny-list (any match rejects)
/// - deny takes precedence when both appear
pub fn platform_fit(node: &PackageNode, host: &HostPlatform) -> PlatformFit {
    let os_ok = matches_constraint(&node.os, host.npm_os());
    let cpu_ok = matches_constraint(&node.cpu, host.npm_cpu());
    if os_ok && cpu_ok {
        return PlatformFit::Compatible;
    }
    if node.optional {
        PlatformFit::SkipOptional
    } else {
        PlatformFit::RejectRequired
    }
}

/// Whether a constraint list accepts `value`.
pub fn matches_constraint(list: &[String], value: &str) -> bool {
    if list.is_empty() {
        return true;
    }
    let mut has_positive = false;
    let mut positive_hit = false;
    for entry in list {
        if let Some(denied) = entry.strip_prefix('!') {
            if denied == value {
                return false;
            }
        } else {
            has_positive = true;
            if entry == value {
                positive_hit = true;
            }
        }
    }
    if has_positive {
        positive_hit
    } else {
        // Only negations present and none matched → allowed.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PackageKey, PackageSource};
    use std::collections::BTreeMap;

    fn node(os: &[&str], cpu: &[&str], optional: bool) -> PackageNode {
        PackageNode {
            key: PackageKey::new("node_modules/x"),
            name: Some("x".into()),
            version: Some("1.0.0".into()),
            source: PackageSource::Registry {
                resolved: "https://example/x.tgz".into(),
            },
            integrity: None,
            dependencies: BTreeMap::new(),
            dev_dependencies: BTreeMap::new(),
            optional_dependencies: BTreeMap::new(),
            peer_dependencies: BTreeMap::new(),
            peer_dependencies_meta: BTreeMap::new(),
            has_install_script: false,
            optional,
            dev: false,
            peer: false,
            cpu: cpu.iter().map(|s| (*s).to_owned()).collect(),
            os: os.iter().map(|s| (*s).to_owned()).collect(),
            bundled_dependencies: Vec::new(),
            is_workspace: false,
            is_link: false,
            likely_native: false,
            bin: BTreeMap::new(),
        }
    }

    #[test]
    fn allow_list_and_negation() {
        let linux = HostPlatform {
            os: "linux".into(),
            arch: "x86_64".into(),
        };
        assert_eq!(
            platform_fit(&node(&["linux"], &["x64"], false), &linux),
            PlatformFit::Compatible
        );
        assert_eq!(
            platform_fit(&node(&["darwin"], &["x64"], true), &linux),
            PlatformFit::SkipOptional
        );
        assert_eq!(
            platform_fit(&node(&["darwin"], &["x64"], false), &linux),
            PlatformFit::RejectRequired
        );
        assert_eq!(
            platform_fit(&node(&["!linux"], &[], false), &linux),
            PlatformFit::RejectRequired
        );
        assert!(matches_constraint(&["!win32".into()], "linux"));
        assert!(!matches_constraint(&["!linux".into()], "linux"));
    }

    #[test]
    fn macos_maps_to_darwin() {
        let mac = HostPlatform {
            os: "macos".into(),
            arch: "aarch64".into(),
        };
        assert_eq!(mac.npm_os(), "darwin");
        assert_eq!(mac.npm_cpu(), "arm64");
        assert_eq!(
            platform_fit(&node(&["darwin"], &["arm64"], false), &mac),
            PlatformFit::Compatible
        );
    }
}
