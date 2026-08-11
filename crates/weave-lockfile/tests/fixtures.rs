//! Golden fixture tests for lockfile → dependency graph parsing.

use std::path::{Path, PathBuf};

use weave_core::{EdgeKind, PackageKey, PackageSource};
use weave_lockfile::parse_lockfile;

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn parse_fixture(name: &str) -> weave_core::DependencyGraph {
    let path = fixtures_root().join(name).join("package-lock.json");
    parse_lockfile(&path).unwrap_or_else(|err| panic!("fixture {name}: {err}"))
}

#[test]
fn flat_tree() {
    let graph = parse_fixture("flat");
    assert_eq!(graph.lockfile_version, 3);
    assert_eq!(graph.package_count(), 2);
    assert!(graph
        .nodes
        .contains_key(&PackageKey::new("node_modules/left-pad")));
    assert!(graph
        .nodes
        .contains_key(&PackageKey::new("node_modules/ms")));
    let id = graph.identity();
    assert_eq!(id.as_str().len(), 64);
    // Determinism: re-parse yields identical identity.
    assert_eq!(id, parse_fixture("flat").identity());
}

#[test]
fn nested_versions_keep_distinct_nodes() {
    let graph = parse_fixture("nested-versions");
    assert_eq!(graph.package_count(), 4);
    let v1 = graph
        .get(&PackageKey::new("node_modules/shared"))
        .unwrap()
        .version
        .as_deref();
    let v2 = graph
        .get(&PackageKey::new(
            "node_modules/parent-b/node_modules/shared",
        ))
        .unwrap()
        .version
        .as_deref();
    assert_eq!(v1, Some("1.0.0"));
    assert_eq!(v2, Some("2.0.0"));

    let edge_to_nested = graph.edges.iter().any(|e| {
        e.from.as_str() == "node_modules/parent-b"
            && e.to.as_str() == "node_modules/parent-b/node_modules/shared"
            && e.name == "shared"
    });
    assert!(edge_to_nested);
}

#[test]
fn peer_dependencies_create_peer_edges() {
    let graph = parse_fixture("peer-deps");
    let ui = graph.get(&PackageKey::new("node_modules/ui-kit")).unwrap();
    assert!(ui.peer_dependencies.contains_key("react"));
    assert!(graph.edges.iter().any(|e| {
        e.kind == EdgeKind::Peer
            && e.from.as_str() == "node_modules/ui-kit"
            && e.to.as_str() == "node_modules/react"
    }));
    let audit = graph.audit_peers();
    assert!(audit.iter().any(|f| {
        f.peer == "react" && matches!(f.status, weave_core::PeerAuditStatus::Satisfied { .. })
    }));
}

#[test]
fn missing_required_peer_is_audited() {
    let graph = parse_fixture("peer-missing");
    let audit = graph.audit_peers();
    assert!(audit.iter().any(|f| {
        matches!(f.status, weave_core::PeerAuditStatus::MissingRequired) && f.peer == "react"
    }));
}

#[test]
fn optional_missing_peer_is_allowed() {
    let graph = parse_fixture("peer-optional-missing");
    let audit = graph.audit_peers();
    assert!(audit.iter().any(|f| {
        matches!(f.status, weave_core::PeerAuditStatus::MissingOptional) && f.peer == "host-api"
    }));
    assert!(!audit
        .iter()
        .any(|f| matches!(f.status, weave_core::PeerAuditStatus::MissingRequired)));
}

#[test]
fn optional_platform_constraints_parsed() {
    let graph = parse_fixture("optional-platform");
    let fs = graph
        .get(&PackageKey::new("node_modules/fsevents"))
        .unwrap();
    assert!(fs.optional);
    assert_eq!(fs.os, vec!["darwin".to_string()]);
    let linux = weave_core::HostPlatform {
        os: "linux".into(),
        arch: "x86_64".into(),
    };
    assert_eq!(
        weave_core::platform_fit(fs, &linux),
        weave_core::PlatformFit::SkipOptional
    );
    let native = graph
        .get(&PackageKey::new("node_modules/linux-only-native"))
        .unwrap();
    assert_eq!(
        weave_core::platform_fit(native, &linux),
        weave_core::PlatformFit::Compatible
    );
}

#[test]
fn optional_dependencies_and_platform_constraints() {
    let graph = parse_fixture("optional-deps");
    let nice = graph
        .get(&PackageKey::new("node_modules/nice-native"))
        .unwrap();
    assert!(nice.optional);
    assert_eq!(nice.os, vec!["linux".to_string()]);
    assert_eq!(nice.cpu, vec!["x64".to_string()]);
    assert!(graph
        .edges
        .iter()
        .any(|e| { e.kind == EdgeKind::Optional && e.name == "nice-native" }));
}

#[test]
fn native_addon_heuristic() {
    let graph = parse_fixture("native-addon");
    let sqlite = graph.get(&PackageKey::new("node_modules/sqlite3")).unwrap();
    assert!(sqlite.has_install_script);
    assert!(sqlite.likely_native);
}

#[test]
fn lifecycle_scripts_flag() {
    let graph = parse_fixture("lifecycle-scripts");
    let scripts = graph.packages_with_install_scripts();
    assert!(scripts.contains("esbuild"));
}

#[test]
fn monorepo_workspace_and_link_nodes() {
    let graph = parse_fixture("monorepo");
    assert!(graph.nodes.contains_key(&PackageKey::new("packages/app")));
    assert!(graph.nodes.contains_key(&PackageKey::new("packages/lib")));
    let app_link = graph
        .get(&PackageKey::new("node_modules/@demo/app"))
        .unwrap();
    assert!(app_link.is_link);
    let app_ws = graph.get(&PackageKey::new("packages/app")).unwrap();
    assert!(app_ws.is_workspace);
    assert_eq!(app_ws.name.as_deref(), Some("@demo/app"));
}

#[test]
fn workspace_packages_lockfile_v2() {
    let graph = parse_fixture("workspace-packages");
    assert_eq!(graph.lockfile_version, 2);
    let ui = graph.get(&PackageKey::new("packages/ui")).unwrap();
    assert_eq!(ui.name.as_deref(), Some("@acme/ui"));
    assert!(ui.is_workspace);
}

#[test]
fn symlinked_local_package() {
    let graph = parse_fixture("symlinked-package");
    let tool = graph
        .get(&PackageKey::new("node_modules/local-tool"))
        .unwrap();
    assert!(tool.is_link);
    assert!(matches!(
        tool.source,
        PackageSource::Path { .. } | PackageSource::Link { .. }
    ));
}

#[test]
fn unusual_scoped_and_bundled_layout() {
    let graph = parse_fixture("unusual-fs");
    assert!(graph
        .nodes
        .contains_key(&PackageKey::new("node_modules/@scoped/pkg")));
    assert!(graph
        .nodes
        .contains_key(&PackageKey::new("node_modules/weird.name")));
    let scoped = graph
        .get(&PackageKey::new("node_modules/@scoped/pkg"))
        .unwrap();
    assert!(scoped
        .bundled_dependencies
        .iter()
        .any(|b| b == "nested-bundle"));
}

#[test]
fn lockfile_v1_nested_tree() {
    let graph = parse_fixture("lockfile-v1");
    assert_eq!(graph.lockfile_version, 1);
    assert!(graph
        .nodes
        .contains_key(&PackageKey::new("node_modules/ms")));
    assert!(graph
        .nodes
        .contains_key(&PackageKey::new("node_modules/parent")));
    assert!(graph
        .nodes
        .contains_key(&PackageKey::new("node_modules/parent/node_modules/child")));
    assert!(graph.edges.iter().any(|e| {
        e.from.as_str() == "node_modules/parent"
            && e.to.as_str() == "node_modules/parent/node_modules/child"
    }));
}

#[test]
fn bin_links_parsed_from_lockfile() {
    let graph = parse_fixture("bin-links");
    let rimrafish = graph
        .get(&PackageKey::new("node_modules/demo-cli"))
        .unwrap();
    assert_eq!(
        rimrafish.bin.get("demo-cli").map(String::as_str),
        Some("cli.js")
    );
    let multi = graph
        .get(&PackageKey::new("node_modules/multi-bin"))
        .unwrap();
    assert_eq!(multi.bin.len(), 2);
    let scoped = graph
        .get(&PackageKey::new("node_modules/@scope/tool"))
        .unwrap();
    assert!(scoped.bin.contains_key("scope-tool"));
    let nested = graph
        .get(&PackageKey::new(
            "node_modules/parent/node_modules/nested-cli",
        ))
        .unwrap();
    assert!(nested.bin.contains_key("nested-cli"));
}

#[test]
fn file_deps_are_path_sources() {
    let graph = parse_fixture("file-deps");
    let local = graph
        .get(&PackageKey::new("node_modules/local-lib"))
        .unwrap();
    assert!(matches!(local.source, PackageSource::Path { .. }));
}

#[test]
fn workspaces_resolve_fixture_has_links() {
    let graph = parse_fixture("workspaces-resolve");
    let a = graph.get(&PackageKey::new("node_modules/@acme/a")).unwrap();
    assert!(a.is_link);
    assert!(matches!(a.source, PackageSource::Link { .. }));
}

#[test]
fn distinct_fixtures_produce_distinct_identities() {
    let flat = parse_fixture("flat").identity();
    let nested = parse_fixture("nested-versions").identity();
    let peer = parse_fixture("peer-deps").identity();
    assert_ne!(flat, nested);
    assert_ne!(flat, peer);
    assert_ne!(nested, peer);
}
