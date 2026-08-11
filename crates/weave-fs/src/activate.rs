//! Transactional activation of a candidate `node_modules` tree.

use std::fs;
use std::path::{Path, PathBuf};

use weave_core::{Error, WEAVE_BACKUP_NODE_MODULES, WEAVE_CANDIDATE_DIR, WEAVE_DIR};

use crate::plan::MaterializationPlan;

/// Result of activating a candidate tree into the project `node_modules`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationReport {
    /// Project `node_modules` path now active.
    pub node_modules: PathBuf,
    /// Whether a previous `node_modules` was replaced.
    pub replaced_existing: bool,
}

/// Validate that every extractable package in `plan` exists under `candidate_root`.
pub fn validate_candidate(
    plan: &MaterializationPlan,
    candidate_root: &Path,
) -> weave_core::Result<()> {
    for pkg in &plan.packages {
        let pkg_path = candidate_root.join(pkg.key.as_str());
        if pkg.link_only {
            if pkg.link_target.is_some() {
                let meta = fs::symlink_metadata(&pkg_path).map_err(|source| Error::Io {
                    path: pkg_path.clone(),
                    source,
                })?;
                if !meta.file_type().is_symlink() {
                    return Err(Error::MaterializationFailed {
                        path: pkg_path,
                        reason: format!(
                            "expected workspace link symlink missing ({})",
                            pkg.name.as_deref().unwrap_or(pkg.key.as_str())
                        ),
                    });
                }
            }
            continue;
        }
        if pkg.artifact_id.is_none() {
            continue;
        }
        if !pkg_path.is_dir() {
            return Err(Error::MaterializationFailed {
                path: pkg_path,
                reason: format!(
                    "expected package directory missing after materialization ({})",
                    pkg.name.as_deref().unwrap_or(pkg.key.as_str())
                ),
            });
        }
    }
    Ok(())
}

/// Atomically replace `{project}/node_modules` with `{project}/.weave/candidate/node_modules`.
///
/// On failure after moving the old tree aside, the previous `node_modules` is restored.
pub fn activate_candidate(project_root: &Path) -> weave_core::Result<ActivationReport> {
    let candidate_nm = project_root
        .join(WEAVE_DIR)
        .join(WEAVE_CANDIDATE_DIR)
        .join("node_modules");
    let active_nm = project_root.join("node_modules");
    let backup_nm = project_root.join(WEAVE_DIR).join(WEAVE_BACKUP_NODE_MODULES);

    if !candidate_nm.is_dir() {
        return Err(Error::MaterializationFailed {
            path: candidate_nm,
            reason: "candidate node_modules does not exist; materialize first".into(),
        });
    }

    // Clear any leftover backup from a previous crash.
    if backup_nm.exists() {
        remove_path(&backup_nm)?;
    }

    let replaced_existing = active_nm.exists();
    if replaced_existing {
        // Ensure parent for backup exists.
        if let Some(parent) = backup_nm.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::rename(&active_nm, &backup_nm).map_err(|source| Error::Io {
            path: active_nm.clone(),
            source,
        })?;
    }

    match fs::rename(&candidate_nm, &active_nm) {
        Ok(()) => {
            if backup_nm.exists() {
                // Best-effort cleanup; a leftover backup is recoverable noise, not corruption.
                let _ = remove_path(&backup_nm);
            }
            // Remove empty candidate dir if possible.
            let candidate_root = project_root.join(WEAVE_DIR).join(WEAVE_CANDIDATE_DIR);
            let _ = fs::remove_dir(&candidate_root);
            Ok(ActivationReport {
                node_modules: active_nm,
                replaced_existing,
            })
        }
        Err(err) => {
            // Restore previous environment.
            if backup_nm.exists() {
                let _ = fs::rename(&backup_nm, &active_nm);
            }
            Err(Error::Io {
                path: active_nm,
                source: err,
            })
        }
    }
}

fn remove_path(path: &Path) -> weave_core::Result<()> {
    let meta = fs::symlink_metadata(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if meta.is_dir() {
        fs::remove_dir_all(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })
    } else {
        fs::remove_file(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use weave_core::PackageKey;
    use weave_store::ContentStore;

    use crate::extract::pack_npm_tarball_for_test;
    use crate::materialize::materialize_plan;
    use crate::plan::{MaterializationPlan, PlannedPackage};

    #[test]
    fn activates_replacing_existing_node_modules() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("node_modules/old")).unwrap();
        fs::write(root.join("node_modules/old/x.txt"), b"old").unwrap();

        let store = ContentStore::open(root.join(".weave-store")).unwrap();
        let tgz = pack_npm_tarball_for_test(&[("index.js", b"exports.n=1;")]);
        let id = store.put(&tgz, None).unwrap();

        let plan = MaterializationPlan {
            packages: vec![PlannedPackage {
                key: PackageKey::new("node_modules/new"),
                name: Some("new".into()),
                artifact_id: Some(id),
                link_only: false,
                prefer_copy: false,
                bins: Default::default(),
                link_target: None,
                likely_native: false,
                has_install_script: false,
            }],
            skipped_optional_platform: Vec::new(),
        };
        let candidate = root.join(WEAVE_DIR).join(WEAVE_CANDIDATE_DIR);
        materialize_plan(&plan, &store, &candidate, root).unwrap();
        validate_candidate(&plan, &candidate).unwrap();

        let report = activate_candidate(root).unwrap();
        assert!(report.replaced_existing);
        assert!(root.join("node_modules/new/index.js").is_file());
        assert!(!root.join("node_modules/old").exists());
        assert!(!root
            .join(WEAVE_DIR)
            .join(WEAVE_BACKUP_NODE_MODULES)
            .exists());
    }

    #[test]
    fn activation_restore_keeps_old_tree_when_candidate_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("node_modules/keep")).unwrap();
        fs::write(root.join("node_modules/keep/x.txt"), b"alive").unwrap();
        // No candidate — activate must fail and leave node_modules intact.
        let err = activate_candidate(root).unwrap_err();
        assert!(matches!(err, Error::MaterializationFailed { .. }));
        assert_eq!(
            fs::read_to_string(root.join("node_modules/keep/x.txt")).unwrap(),
            "alive"
        );
    }
}
