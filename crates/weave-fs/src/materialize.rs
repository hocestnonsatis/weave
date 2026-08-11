//! Apply a [`MaterializationPlan`] to a destination directory.

use std::fs;
use std::path::Path;

use weave_core::Error;
use weave_store::ContentStore;

use crate::bins::link_package_bins;
use crate::link::{link_or_copy_tree, same_filesystem, LinkStats};
use crate::plan::MaterializationPlan;
use crate::unpacked::UnpackedCache;
use crate::workspace::wire_workspace_links;

/// Summary of a materialization run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializeReport {
    /// Packages placed into the destination tree.
    pub packages_materialized: usize,
    /// Link-only / skipped packages (before workspace wiring).
    pub skipped_links: usize,
    /// Unpacked-cache hits (tarball already extracted).
    pub cache_hits: usize,
    /// Unpacked-cache misses (fresh extract into cache).
    pub cache_misses: usize,
    /// Files hardlinked from the unpacked cache.
    pub hardlinked_files: usize,
    /// Files copied from the unpacked cache.
    pub copied_files: usize,
    /// `.bin` symlinks created.
    pub bin_links: usize,
    /// Workspace/link symlinks created.
    pub workspace_links: usize,
    /// Destination root that was written.
    pub dest: std::path::PathBuf,
}

impl MaterializeReport {
    /// Backward-compatible alias used by older call sites/tests.
    pub fn extracted(&self) -> usize {
        self.packages_materialized
    }
}

/// Materialize `plan` into `dest` using artifacts from `store`.
///
/// Packages are extracted once into an unpacked CAS cache beside the object
/// store, then hardlinked (or copied) into `dest`. Packages marked
/// [`crate::plan::PlannedPackage::prefer_copy`] always copy so install-time
/// mutation cannot corrupt the shared cache.
///
/// `dest` should be an empty candidate directory (e.g. `.weave/candidate`).
/// `project_root` is used to resolve and wire workspace/`file:` link targets.
/// This does **not** activate the environment.
pub fn materialize_plan(
    plan: &MaterializationPlan,
    store: &ContentStore,
    dest: &Path,
    project_root: &Path,
) -> weave_core::Result<MaterializeReport> {
    fs::create_dir_all(dest).map_err(|source| Error::Io {
        path: dest.to_path_buf(),
        source,
    })?;

    let cache = UnpackedCache::for_store(store);
    let mut packages_materialized = 0;
    let mut skipped_links = 0;
    let mut cache_hits = 0;
    let mut cache_misses = 0;
    let mut link_totals = LinkStats::default();

    for pkg in &plan.packages {
        let pkg_dest = dest.join(pkg.key.as_str());
        if pkg.link_only || pkg.artifact_id.is_none() {
            skipped_links += 1;
            continue;
        }
        let id = pkg.artifact_id.as_ref().unwrap();
        let (unpacked, hit) = cache.ensure(store, id)?;
        if hit {
            cache_hits += 1;
        } else {
            cache_misses += 1;
        }

        if pkg_dest.exists() {
            fs::remove_dir_all(&pkg_dest).map_err(|source| Error::Io {
                path: pkg_dest.clone(),
                source,
            })?;
        }

        let can_hardlink = !pkg.prefer_copy && same_filesystem(&unpacked, dest);
        let stats = link_or_copy_tree(&unpacked, &pkg_dest, can_hardlink)?;
        link_totals.hardlinked_files += stats.hardlinked_files;
        link_totals.copied_files += stats.copied_files;
        link_totals.directories_created += stats.directories_created;
        link_totals.symlinks_created += stats.symlinks_created;
        packages_materialized += 1;
    }

    let workspace = wire_workspace_links(&plan.packages, dest, project_root)?;
    let bins = link_package_bins(&plan.packages, dest)?;

    Ok(MaterializeReport {
        packages_materialized,
        skipped_links,
        cache_hits,
        cache_misses,
        hardlinked_files: link_totals.hardlinked_files,
        copied_files: link_totals.copied_files,
        bin_links: bins.links_created,
        workspace_links: workspace.links_created,
        dest: dest.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use weave_core::{DependencyGraph, LockfileKind, PackageKey, PackageNode, PackageSource};
    use weave_store::{hash_bytes, ContentStore};

    use crate::extract::pack_npm_tarball_for_test;
    use crate::plan::MaterializationPlan;

    #[test]
    fn materializes_flat_tree_from_store() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::open(tmp.path().join("store").join("objects")).unwrap();

        let tgz_a = pack_npm_tarball_for_test(&[("index.js", b"exports.a=1;")]);
        let tgz_b = pack_npm_tarball_for_test(&[("index.js", b"exports.b=1;")]);
        let id_a = store.put(&tgz_a, None).unwrap();
        let id_b = store.put(&tgz_b, None).unwrap();
        assert_eq!(id_a, hash_bytes(&tgz_a));

        let mut nodes = BTreeMap::new();
        nodes.insert(PackageKey::root(), empty_root());
        nodes.insert(
            PackageKey::new("node_modules/a"),
            pkg("a", "https://example/a.tgz"),
        );
        nodes.insert(
            PackageKey::new("node_modules/b"),
            pkg("b", "https://example/b.tgz"),
        );
        let graph = DependencyGraph {
            lockfile_kind: LockfileKind::NpmPackageLock,
            lockfile_version: 3,
            root: PackageKey::root(),
            nodes,
            edges: Vec::new(),
        };

        let mut artifacts = BTreeMap::new();
        artifacts.insert(PackageKey::new("node_modules/a"), id_a.clone());
        artifacts.insert(PackageKey::new("node_modules/b"), id_b);

        let plan = MaterializationPlan::from_graph(&graph, &artifacts);
        let dest = tmp.path().join("candidate");
        let report = materialize_plan(&plan, &store, &dest, tmp.path()).unwrap();
        assert_eq!(report.packages_materialized, 2);
        assert_eq!(report.cache_misses, 2);
        assert_eq!(
            fs::read_to_string(dest.join("node_modules/a/index.js")).unwrap(),
            "exports.a=1;"
        );

        let dest2 = tmp.path().join("candidate2");
        let report2 = materialize_plan(&plan, &store, &dest2, tmp.path()).unwrap();
        assert_eq!(report2.cache_hits, 2);
        assert_eq!(report2.cache_misses, 0);
        #[cfg(unix)]
        {
            assert!(report2.hardlinked_files >= 2);
        }
    }

    #[test]
    fn native_and_install_script_packages_prefer_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::open(tmp.path().join("store").join("objects")).unwrap();
        let tgz = pack_npm_tarball_for_test(&[("index.js", b"exports.n=1;")]);
        let id = store.put(&tgz, None).unwrap();

        let mut nodes = BTreeMap::new();
        nodes.insert(PackageKey::root(), empty_root());
        let mut native = pkg("native-addon", "https://example/n.tgz");
        native.has_install_script = true;
        native.likely_native = true;
        native.cpu = vec!["x64".into()];
        native.os = vec!["linux".into()];
        nodes.insert(PackageKey::new("node_modules/native-addon"), native);
        let graph = DependencyGraph {
            lockfile_kind: LockfileKind::NpmPackageLock,
            lockfile_version: 3,
            root: PackageKey::root(),
            nodes,
            edges: Vec::new(),
        };
        let mut artifacts = BTreeMap::new();
        artifacts.insert(PackageKey::new("node_modules/native-addon"), id);
        let plan = MaterializationPlan::from_graph(&graph, &artifacts);
        assert!(plan.packages[0].prefer_copy);

        let dest = tmp.path().join("candidate");
        let report = materialize_plan(&plan, &store, &dest, tmp.path()).unwrap();
        assert_eq!(report.packages_materialized, 1);
        assert!(report.copied_files >= 1);
        assert_eq!(report.hardlinked_files, 0);
    }

    #[test]
    fn creates_bin_symlinks_for_declared_bins() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::open(tmp.path().join("store").join("objects")).unwrap();
        let tgz = pack_npm_tarball_for_test(&[
            (
                "package.json",
                br#"{"name":"demo-cli","version":"1.0.0","bin":{"demo-cli":"cli.js"}}"#,
            ),
            ("cli.js", b"#!/usr/bin/env node\nconsole.log('ok');\n"),
            ("index.js", b"module.exports = 1;\n"),
        ]);
        let id = store.put(&tgz, None).unwrap();
        let mut nodes = BTreeMap::new();
        nodes.insert(PackageKey::root(), empty_root());
        let mut node = pkg("demo-cli", "https://example/cli.tgz");
        node.bin.insert("demo-cli".into(), "cli.js".into());
        nodes.insert(PackageKey::new("node_modules/demo-cli"), node);
        let graph = DependencyGraph {
            lockfile_kind: LockfileKind::NpmPackageLock,
            lockfile_version: 3,
            root: PackageKey::root(),
            nodes,
            edges: Vec::new(),
        };
        let mut artifacts = BTreeMap::new();
        artifacts.insert(PackageKey::new("node_modules/demo-cli"), id);
        let plan = MaterializationPlan::from_graph(&graph, &artifacts);
        let dest = tmp.path().join("candidate");
        let report = materialize_plan(&plan, &store, &dest, tmp.path()).unwrap();
        assert_eq!(report.bin_links, 1);
        let link = dest.join("node_modules/.bin/demo-cli");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        let target = fs::read_link(&link).unwrap();
        assert_eq!(target, Path::new("../demo-cli/cli.js"));
    }

    fn pkg(name: &str, url: &str) -> PackageNode {
        PackageNode {
            key: PackageKey::new(format!("node_modules/{name}")),
            name: Some(name.into()),
            version: Some("1.0.0".into()),
            source: PackageSource::Registry {
                resolved: url.into(),
            },
            integrity: None,
            dependencies: BTreeMap::new(),
            dev_dependencies: BTreeMap::new(),
            optional_dependencies: BTreeMap::new(),
            peer_dependencies: BTreeMap::new(),
            peer_dependencies_meta: BTreeMap::new(),
            has_install_script: false,
            optional: false,
            dev: false,
            peer: false,
            cpu: Vec::new(),
            os: Vec::new(),
            bundled_dependencies: Vec::new(),
            is_workspace: false,
            is_link: false,
            likely_native: false,
            bin: BTreeMap::new(),
        }
    }

    fn empty_root() -> PackageNode {
        PackageNode {
            key: PackageKey::root(),
            name: Some("root".into()),
            version: Some("1.0.0".into()),
            source: PackageSource::Workspace,
            integrity: None,
            dependencies: BTreeMap::new(),
            dev_dependencies: BTreeMap::new(),
            optional_dependencies: BTreeMap::new(),
            peer_dependencies: BTreeMap::new(),
            peer_dependencies_meta: BTreeMap::new(),
            has_install_script: false,
            optional: false,
            dev: false,
            peer: false,
            cpu: Vec::new(),
            os: Vec::new(),
            bundled_dependencies: Vec::new(),
            is_workspace: true,
            is_link: false,
            likely_native: false,
            bin: BTreeMap::new(),
        }
    }
}
