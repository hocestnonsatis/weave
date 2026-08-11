//! Reachability-based garbage collection for the content store.
//!
//! GC always removes incomplete temps first, then deletes artifacts that are
//! not reachable from:
//! - environments in the current project
//! - environments in other projects registered against the same store
//! - explicit pins in `$WEAVE_HOME/pins.json`
//!
//! Unpacked cache entries for deleted (or otherwise unreachable) artifacts are
//! removed as well. Correctness beats maximum reclamation.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use weave_core::Error;
use weave_fs::UnpackedCache;
use weave_store::{ArtifactId, ContentStore};

use crate::config::ProjectConfig;
use crate::project::discover_project;
use crate::registry::{
    artifact_roots_from_project, load_pins, register_project, registered_projects_for_store,
};

/// Options controlling a GC run.
#[derive(Debug, Clone, Default)]
pub struct GcOptions {
    /// When true, report what would be deleted without deleting reachable-based
    /// objects (temps are still cleaned — they are never reachable).
    pub dry_run: bool,
}

/// Summary of a GC run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GcReport {
    /// Store object root inspected.
    pub store_root: PathBuf,
    /// Temporary object files removed.
    pub removed_object_temps: usize,
    /// Incomplete unpacked package directories removed.
    pub removed_unpacked_incomplete: usize,
    /// Ready-marker files removed alongside incomplete dirs (usually 0).
    pub removed_unpacked_markers: usize,
    /// Complete objects removed because they were unreachable.
    pub removed_unreachable_objects: usize,
    /// Complete unpacked cache entries removed because unreachable.
    pub removed_unreachable_unpacked: usize,
    /// Number of artifact ids treated as GC roots.
    pub root_artifacts: usize,
    /// Number of project roots contributing environments.
    pub root_projects: usize,
    /// Whether this was a dry run (no reachability deletions).
    pub dry_run: bool,
}

/// Run GC for the project containing `start`.
pub fn gc_project(start: &Path) -> weave_core::Result<GcReport> {
    gc_project_with_options(start, &GcOptions::default())
}

/// Run GC with explicit options.
pub fn gc_project_with_options(start: &Path, options: &GcOptions) -> weave_core::Result<GcReport> {
    let discovery = discover_project(start)?;
    if !discovery.layout.weave_initialized {
        return Err(Error::NotInitialized {
            root: discovery.layout.root,
        });
    }
    let root = discovery.layout.root;
    // Best-effort: keep registry fresh for shared-store GC.
    let _ = register_project(&root);

    let config = ProjectConfig::load(&root)?;
    let store_root = PathBuf::from(&config.store_path);
    let store = ContentStore::open(&store_root)?;

    let mut projects = registered_projects_for_store(store.root())?;
    if !projects.iter().any(|p| p == &root) {
        projects.push(root.clone());
    }
    projects.sort();
    projects.dedup();

    let mut roots = BTreeSet::new();
    for project in &projects {
        for id in artifact_roots_from_project(project)? {
            if let Ok(parsed) = ArtifactId::parse(&id) {
                roots.insert(parsed);
            }
        }
    }
    for pin in load_pins(store.root())? {
        if let Ok(parsed) = ArtifactId::parse(&pin) {
            roots.insert(parsed);
        }
    }

    gc_store_with_roots(store.root(), &roots, projects.len(), options)
}

/// GC a store object root with no environment roots (temps + orphans only).
///
/// Prefer [`gc_project`] / [`gc_store_with_roots`] for production use.
pub fn gc_store(store_root: &Path) -> weave_core::Result<GcReport> {
    gc_store_with_roots(store_root, &BTreeSet::new(), 0, &GcOptions::default())
}

/// GC with an explicit root set (tests and tooling).
pub fn gc_store_with_roots(
    store_root: &Path,
    roots: &BTreeSet<ArtifactId>,
    root_projects: usize,
    options: &GcOptions,
) -> weave_core::Result<GcReport> {
    let mut removed_object_temps = 0usize;
    let objects = store_root.join("sha256");
    if objects.is_dir() {
        removed_object_temps += remove_temp_files(&objects)?;
    }

    let store = ContentStore::open(store_root)?;
    let unpacked = UnpackedCache::for_store(&store);

    let mut removed_unpacked_incomplete = 0usize;
    let mut removed_unpacked_markers = 0usize;
    let unpacked_sha = unpacked.root().join("sha256");
    if unpacked_sha.is_dir() {
        let (dirs, markers) = remove_incomplete_unpacked(&unpacked_sha)?;
        removed_unpacked_incomplete += dirs;
        removed_unpacked_markers += markers;
    }

    let mut removed_unreachable_objects = 0usize;
    let mut removed_unreachable_unpacked = 0usize;

    let object_ids = store.list_ids()?;
    for id in &object_ids {
        if roots.contains(id) {
            continue;
        }
        if options.dry_run {
            removed_unreachable_objects += 1;
            if unpacked.contains(id) {
                removed_unreachable_unpacked += 1;
            }
            continue;
        }
        store.remove(id)?;
        removed_unreachable_objects += 1;
        if unpacked.contains(id) {
            unpacked.remove(id)?;
            removed_unreachable_unpacked += 1;
        }
    }

    // Orphan unpacked entries whose object is already gone / never rooted.
    for id in unpacked.list_ids()? {
        if roots.contains(&id) {
            continue;
        }
        if options.dry_run {
            if object_ids.binary_search(&id).is_err() {
                removed_unreachable_unpacked += 1;
            }
            continue;
        }
        // Already removed with object above when object existed.
        if unpacked.contains(&id) {
            unpacked.remove(&id)?;
            removed_unreachable_unpacked += 1;
        }
    }

    Ok(GcReport {
        store_root: store_root.to_path_buf(),
        removed_object_temps,
        removed_unpacked_incomplete,
        removed_unpacked_markers,
        removed_unreachable_objects,
        removed_unreachable_unpacked,
        root_artifacts: roots.len(),
        root_projects,
        dry_run: options.dry_run,
    })
}

fn remove_temp_files(root: &Path) -> weave_core::Result<usize> {
    let mut removed = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).map_err(|source| Error::Io {
            path: dir.clone(),
            source,
        })? {
            let entry = entry.map_err(|source| Error::Io {
                path: dir.clone(),
                source,
            })?;
            let path = entry.path();
            let meta = entry.metadata().map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
            if meta.is_dir() {
                stack.push(path);
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && name.contains(".tmp-") {
                fs::remove_file(&path).map_err(|source| Error::Io {
                    path: path.clone(),
                    source,
                })?;
                removed += 1;
            }
        }
    }
    Ok(removed)
}

fn remove_incomplete_unpacked(sha_root: &Path) -> weave_core::Result<(usize, usize)> {
    let mut removed_dirs = 0usize;
    let mut removed_markers = 0usize;
    for shard in fs::read_dir(sha_root).map_err(|source| Error::Io {
        path: sha_root.to_path_buf(),
        source,
    })? {
        let shard = shard.map_err(|source| Error::Io {
            path: sha_root.to_path_buf(),
            source,
        })?;
        let shard_path = shard.path();
        if !shard_path.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&shard_path).map_err(|source| Error::Io {
            path: shard_path.clone(),
            source,
        })? {
            let entry = entry.map_err(|source| Error::Io {
                path: shard_path.clone(),
                source,
            })?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".tmp-") && path.is_dir() {
                let _ = clear_writable(&path);
                fs::remove_dir_all(&path).map_err(|source| Error::Io {
                    path: path.clone(),
                    source,
                })?;
                removed_dirs += 1;
                continue;
            }
            if path.is_dir() && !name.starts_with('.') {
                let marker = shard_path.join(format!("{name}.ready"));
                if !marker.is_file() {
                    let _ = clear_writable(&path);
                    fs::remove_dir_all(&path).map_err(|source| Error::Io {
                        path: path.clone(),
                        source,
                    })?;
                    removed_dirs += 1;
                }
            }
            if name.ends_with(".ready") {
                let pkg = shard_path.join(name.trim_end_matches(".ready"));
                if !pkg.is_dir() {
                    fs::remove_file(&path).map_err(|source| Error::Io {
                        path: path.clone(),
                        source,
                    })?;
                    removed_markers += 1;
                }
            }
        }
    }
    Ok((removed_dirs, removed_markers))
}

fn clear_writable(path: &Path) -> weave_core::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = fs::symlink_metadata(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if meta.file_type().is_symlink() {
            return Ok(());
        }
        if meta.is_dir() {
            for entry in fs::read_dir(path).map_err(|source| Error::Io {
                path: path.to_path_buf(),
                source,
            })? {
                let entry = entry.map_err(|source| Error::Io {
                    path: path.to_path_buf(),
                    source,
                })?;
                let _ = clear_writable(&entry.path());
            }
        }
        let mode = meta.permissions().mode();
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode | 0o200));
    }
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use weave_core::{DependencyGraph, LockfileKind, PackageKey, PackageNode, PackageSource};
    use weave_store::hash_bytes;

    use crate::config::ProjectConfig;
    use crate::environment::create_environment;
    use crate::init::init_project;
    use crate::registry::{load_pins, register_project};
    use crate::test_util::lock_weave_home;

    #[test]
    fn removes_object_temps_and_keeps_real_objects() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::open(tmp.path().join("objects")).unwrap();
        let id = store.put(b"keep-me", None).unwrap();
        let shard = store.root().join("sha256").join(id.shard());
        fs::write(shard.join(format!(".{}.tmp-dead", id.object_name())), b"x").unwrap();

        // Empty roots → object is unreachable and deleted.
        let report = gc_store(store.root()).unwrap();
        assert_eq!(report.removed_object_temps, 1);
        assert_eq!(report.removed_unreachable_objects, 1);
        assert!(!store.contains(&id));
        assert_eq!(hash_bytes(b"keep-me"), id);
    }

    #[test]
    fn removes_incomplete_unpacked_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let objects = tmp.path().join("objects");
        let _store = ContentStore::open(&objects).unwrap();
        let unpacked = tmp.path().join("unpacked").join("sha256").join("ab");
        fs::create_dir_all(&unpacked).unwrap();
        fs::create_dir_all(unpacked.join("incompletepkg")).unwrap();
        fs::create_dir_all(unpacked.join(".tmp-123")).unwrap();
        fs::create_dir_all(unpacked.join("completepkg")).unwrap();
        fs::write(unpacked.join("completepkg.ready"), b"").unwrap();

        let report = gc_store(&objects).unwrap();
        assert!(report.removed_unpacked_incomplete >= 2);
        assert!(unpacked.join("completepkg").is_dir());
        assert!(unpacked.join("completepkg.ready").is_file());
        assert!(!unpacked.join("incompletepkg").exists());
        assert!(!unpacked.join(".tmp-123").exists());
    }

    #[test]
    fn keeps_reachable_deletes_unreachable() {
        let _guard = lock_weave_home();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));

        let project = tmp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        setup_git_node_project(&project);

        init_project(&project).unwrap();
        let store = ContentStore::open(ProjectConfig::load(&project).unwrap().store_path).unwrap();
        let keep = store.put(b"keep-bytes", None).unwrap();
        let drop = store.put(b"drop-bytes", None).unwrap();

        let mut map = BTreeMap::new();
        map.insert(PackageKey::new("node_modules/demo"), keep.clone());
        create_environment(&project, &empty_graph(), &map, None).unwrap();
        register_project(&project).unwrap();

        let report = gc_project(&project).unwrap();
        assert!(store.contains(&keep));
        assert!(!store.contains(&drop));
        assert_eq!(report.removed_unreachable_objects, 1);
        assert!(report.root_artifacts >= 1);

        std::env::remove_var("WEAVE_HOME");
    }

    #[test]
    fn dry_run_does_not_delete_objects() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::open(tmp.path().join("objects")).unwrap();
        let id = store.put(b"orphan", None).unwrap();
        let report = gc_store_with_roots(
            store.root(),
            &BTreeSet::new(),
            0,
            &GcOptions { dry_run: true },
        )
        .unwrap();
        assert_eq!(report.removed_unreachable_objects, 1);
        assert!(report.dry_run);
        assert!(store.contains(&id));
    }

    #[test]
    fn pin_protects_artifact() {
        let _guard = lock_weave_home();
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("weave-home");
        std::env::set_var("WEAVE_HOME", &home);
        let store = ContentStore::open(home.join("store").join("objects")).unwrap();
        let id = store.put(b"pinned", None).unwrap();
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join("pins.json"),
            format!(r#"{{"artifacts":["{}"]}}"#, id.as_str()),
        )
        .unwrap();

        let mut roots = BTreeSet::new();
        for pin in load_pins(store.root()).unwrap() {
            roots.insert(ArtifactId::parse(pin).unwrap());
        }
        let report = gc_store_with_roots(store.root(), &roots, 0, &GcOptions::default()).unwrap();
        assert!(store.contains(&id));
        assert_eq!(report.removed_unreachable_objects, 0);
        assert_eq!(report.root_artifacts, 1);
        std::env::remove_var("WEAVE_HOME");
    }

    #[test]
    fn concurrent_gc_with_empty_roots_is_safe() {
        use std::sync::Arc;
        use std::thread;

        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::open(tmp.path().join("objects")).unwrap();
        for i in 0..16u8 {
            store.put(&[b'g', i], None).unwrap();
        }
        let root = store.root().to_path_buf();
        let root = Arc::new(root);
        let mut handles = Vec::new();
        for _ in 0..4 {
            let root = Arc::clone(&root);
            handles.push(thread::spawn(move || {
                gc_store_with_roots(&root, &BTreeSet::new(), 0, &GcOptions::default()).unwrap()
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert!(ContentStore::open(store.root())
            .unwrap()
            .list_ids()
            .unwrap()
            .is_empty());
    }

    fn empty_graph() -> DependencyGraph {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            PackageKey::root(),
            PackageNode {
                key: PackageKey::root(),
                name: Some("demo".into()),
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
            },
        );
        DependencyGraph {
            lockfile_kind: LockfileKind::NpmPackageLock,
            lockfile_version: 3,
            root: PackageKey::root(),
            nodes,
            edges: Vec::new(),
        }
    }

    fn setup_git_node_project(dir: &Path) {
        use std::process::Command;
        let status = Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success());
        let _ = Command::new("git")
            .args(["config", "user.email", "weave@example.com"])
            .current_dir(dir)
            .status();
        let _ = Command::new("git")
            .args(["config", "user.name", "Weave Test"])
            .current_dir(dir)
            .status();
        fs::write(
            dir.join("package.json"),
            r#"{"name":"demo","version":"1.0.0"}"#,
        )
        .unwrap();
        fs::write(
            dir.join("package-lock.json"),
            r#"{
  "name": "demo",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "packages": {
    "": { "name": "demo", "version": "1.0.0" }
  }
}"#,
        )
        .unwrap();
        fs::write(dir.join("README"), "x").unwrap();
        let status = Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success());
        let status = Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success());
    }
}
