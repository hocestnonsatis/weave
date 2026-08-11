//! `node_modules/.bin` linking for package executables.
//!
//! # Linux behavior
//!
//! Weave follows npm's Unix strategy: create a **relative symlink** from
//! `…/node_modules/.bin/<name>` to the package's bin script
//! (`../<pkg>/<script>`). No cmd-shim wrapper is written on Linux.
//!
//! The target script's executable bit is set (`chmod u+x`) when Weave can
//! change permissions so `node_modules/.bin/<name>` is directly executable.
//!
//! Windows cmd/ps1 shims are **not** implemented (unsupported on that OS).
//!
//! Conflicting bin names: later packages in plan order (sorted by package
//! key) overwrite earlier links. This matches deterministic last-writer-wins.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use weave_core::Error;

use crate::plan::PlannedPackage;

/// Summary of `.bin` linking for one materialization.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BinLinkReport {
    /// Symlinks (or replacements) created under `.bin` directories.
    pub links_created: usize,
    /// Bin names that overwrote an existing link.
    pub conflicts_overwritten: usize,
    /// Packages whose declared bin target was missing on disk (skipped).
    pub missing_targets: usize,
}

/// Link all declared bins for materialized packages under `dest`.
///
/// `dest` is the candidate root (contains `node_modules/…` package keys).
pub fn link_package_bins(
    packages: &[PlannedPackage],
    dest: &Path,
) -> weave_core::Result<BinLinkReport> {
    let mut report = BinLinkReport::default();
    for pkg in packages {
        if pkg.link_only && pkg.artifact_id.is_none() && pkg.link_target.is_none() {
            continue;
        }
        let pkg_dir = dest.join(pkg.key.as_str());
        if !pkg_dir.exists() {
            continue;
        }

        let mut bins = pkg.bins.clone();
        if bins.is_empty() {
            bins = read_bins_from_package_json(&pkg_dir, pkg.name.as_deref())?;
        }
        if bins.is_empty() {
            continue;
        }

        let bin_dir_rel = nearest_bin_dir(pkg.key.as_str());
        let bin_dir = dest.join(&bin_dir_rel);
        fs::create_dir_all(&bin_dir).map_err(|source| Error::Io {
            path: bin_dir.clone(),
            source,
        })?;

        let package_name_in_nm = package_name_in_node_modules(pkg.key.as_str());
        for (bin_name, bin_rel) in bins {
            let bin_name = sanitize_bin_name(&bin_name)?;
            let bin_rel = sanitize_bin_rel(&bin_rel)?;
            let target_abs = pkg_dir.join(&bin_rel);
            if !target_abs.is_file() {
                report.missing_targets += 1;
                continue;
            }
            ensure_executable(&target_abs)?;

            let link_path = bin_dir.join(&bin_name);
            if link_path.exists() || link_path.symlink_metadata().is_ok() {
                report.conflicts_overwritten += 1;
                remove_any(&link_path)?;
            }

            // Relative from `.bin/` to the script: `../<pkg>/<bin_rel>`
            let link_target = PathBuf::from(format!("../{package_name_in_nm}/{bin_rel}"));
            create_relative_symlink(&link_target, &link_path)?;
            report.links_created += 1;
        }
    }
    Ok(report)
}

fn nearest_bin_dir(package_key: &str) -> String {
    let Some((prefix, _)) = package_key.rsplit_once("node_modules/") else {
        return "node_modules/.bin".into();
    };
    if prefix.is_empty() {
        "node_modules/.bin".into()
    } else {
        format!("{prefix}node_modules/.bin")
    }
}

fn package_name_in_node_modules(package_key: &str) -> &str {
    package_key
        .rsplit_once("node_modules/")
        .map(|(_, name)| name)
        .unwrap_or(package_key)
}

fn sanitize_bin_name(name: &str) -> weave_core::Result<String> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
        || name == "."
        || name == ".."
    {
        return Err(Error::MaterializationFailed {
            path: PathBuf::from(name),
            reason: "invalid bin name".into(),
        });
    }
    Ok(name.to_owned())
}

fn sanitize_bin_rel(rel: &str) -> weave_core::Result<String> {
    let path = Path::new(rel);
    if path.is_absolute() {
        return Err(Error::MaterializationFailed {
            path: path.to_path_buf(),
            reason: "absolute bin paths are not allowed".into(),
        });
    }
    for comp in path.components() {
        match comp {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::MaterializationFailed {
                    path: path.to_path_buf(),
                    reason: "bin path traversal rejected".into(),
                });
            }
        }
    }
    Ok(rel.replace('\\', "/"))
}

fn read_bins_from_package_json(
    pkg_dir: &Path,
    package_name: Option<&str>,
) -> weave_core::Result<BTreeMap<String, String>> {
    let path = pkg_dir.join("package.json");
    if !path.is_file() {
        return Ok(BTreeMap::new());
    }
    let bytes = fs::read(&path).map_err(|source| Error::Io {
        path: path.clone(),
        source,
    })?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|err| Error::InvalidState {
            path: path.clone(),
            reason: format!("invalid package.json: {err}"),
        })?;
    Ok(match value.get("bin") {
        Some(serde_json::Value::String(p)) => {
            let Some(name) = package_name else {
                return Ok(BTreeMap::new());
            };
            let bin_name = name.rsplit('/').next().unwrap_or(name);
            let mut map = BTreeMap::new();
            map.insert(bin_name.to_owned(), p.clone());
            map
        }
        Some(serde_json::Value::Object(obj)) => {
            let mut map = BTreeMap::new();
            for (k, v) in obj {
                if let Some(p) = v.as_str() {
                    map.insert(k.clone(), p.to_owned());
                }
            }
            map
        }
        _ => BTreeMap::new(),
    })
}

fn ensure_executable(path: &Path) -> weave_core::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::fs::PermissionsExt;
        let meta = fs::metadata(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let mode = meta.permissions().mode();
        if mode & 0o111 != 0 {
            return Ok(());
        }
        // Avoid chmod on a hardlinked shared-cache inode: break the link first.
        if meta.nlink() > 1 {
            let bytes = fs::read(path).map_err(|source| Error::Io {
                path: path.to_path_buf(),
                source,
            })?;
            let tmp = path.with_extension("weave-bin-tmp");
            fs::write(&tmp, &bytes).map_err(|source| Error::Io {
                path: tmp.clone(),
                source,
            })?;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755)).map_err(|source| {
                Error::Io {
                    path: tmp.clone(),
                    source,
                }
            })?;
            fs::rename(&tmp, path).map_err(|source| Error::Io {
                path: path.to_path_buf(),
                source,
            })?;
            return Ok(());
        }
        let mut perms = meta.permissions();
        perms.set_mode(mode | 0o755);
        fs::set_permissions(path, perms).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
    }
    let _ = path;
    Ok(())
}

fn remove_any(path: &Path) -> weave_core::Result<()> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(Error::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if meta.file_type().is_dir() && !meta.file_type().is_symlink() {
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

fn create_relative_symlink(target: &Path, link: &Path) -> weave_core::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(target, link).map_err(|source| Error::Io {
            path: link.to_path_buf(),
            source,
        })?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = target;
        Err(Error::NotImplemented(
            "creating package bin symlinks on this platform",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_bin_dir_for_scoped_and_nested() {
        assert_eq!(nearest_bin_dir("node_modules/rimraf"), "node_modules/.bin");
        assert_eq!(
            nearest_bin_dir("node_modules/@scope/cli"),
            "node_modules/.bin"
        );
        assert_eq!(
            nearest_bin_dir("node_modules/a/node_modules/b"),
            "node_modules/a/node_modules/.bin"
        );
    }

    #[test]
    fn rejects_traversal_in_bin_paths() {
        assert!(sanitize_bin_rel("../etc/passwd").is_err());
        assert!(sanitize_bin_name("../x").is_err());
        assert!(sanitize_bin_name("a/b").is_err());
    }
}
