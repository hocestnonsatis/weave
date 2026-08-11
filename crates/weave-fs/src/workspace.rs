//! Workspace / link package wiring into `node_modules`.

use std::fs;
use std::path::{Component, Path, PathBuf};

use weave_core::Error;

use crate::plan::PlannedPackage;

/// Summary of workspace/link wiring.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WorkspaceLinkReport {
    /// Symlinks created for link/workspace packages.
    pub links_created: usize,
    /// Link packages skipped (missing target / not under project).
    pub skipped: usize,
}

/// Create symlinks for link-only packages that point at local workspace paths.
///
/// Symlink targets are **relative** paths that resolve correctly after the
/// candidate `node_modules` is activated into `{project}/node_modules`
/// (npm-compatible layout). Absolute targets are rejected.
pub fn wire_workspace_links(
    packages: &[PlannedPackage],
    dest: &Path,
    project_root: &Path,
) -> weave_core::Result<WorkspaceLinkReport> {
    let mut report = WorkspaceLinkReport::default();
    for pkg in packages {
        let Some(target_rel) = pkg.link_target.as_deref() else {
            continue;
        };
        let target_rel = sanitize_workspace_target(target_rel)?;
        let target_abs = project_root.join(&target_rel);
        if !target_abs.exists() {
            report.skipped += 1;
            continue;
        }

        let link_path = dest.join(pkg.key.as_str());
        if let Some(parent) = link_path.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        if link_path.exists() || link_path.symlink_metadata().is_ok() {
            remove_any(&link_path)?;
        }

        // Relative from the final install location (package key path) to the
        // workspace directory. Uses the key as if rooted at project/, matching
        // post-activation layout.
        let link_target = relative_path(Path::new(pkg.key.as_str()), Path::new(&target_rel))?;
        create_symlink(&link_target, &link_path)?;
        report.links_created += 1;
        let _ = dest;
    }
    Ok(report)
}

fn sanitize_workspace_target(target: &str) -> weave_core::Result<String> {
    let trimmed = target.trim_start_matches("./");
    let path = Path::new(trimmed);
    if path.is_absolute() {
        return Err(Error::MaterializationFailed {
            path: path.to_path_buf(),
            reason: "absolute workspace link targets are not allowed".into(),
        });
    }
    for comp in path.components() {
        match comp {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::MaterializationFailed {
                    path: path.to_path_buf(),
                    reason: "workspace link target escapes project root".into(),
                });
            }
        }
    }
    Ok(trimmed.replace('\\', "/"))
}

/// Compute a relative symlink target from `from_key` (e.g. `node_modules/@a/b`)
/// to `to` (e.g. `packages/b`), as used from the symlink's parent directory.
fn relative_path(from_key: &Path, to: &Path) -> weave_core::Result<PathBuf> {
    let from_parent = from_key.parent().unwrap_or(Path::new(""));
    let from_comps: Vec<_> = from_parent.components().collect();
    let to_comps: Vec<_> = to.components().collect();

    let mut i = 0;
    while i < from_comps.len()
        && i < to_comps.len()
        && from_comps[i].as_os_str() == to_comps[i].as_os_str()
    {
        i += 1;
    }

    let mut out = PathBuf::new();
    for _ in i..from_comps.len() {
        out.push("..");
    }
    for comp in &to_comps[i..] {
        out.push(comp);
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    Ok(out)
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

fn create_symlink(target: &Path, link: &Path) -> weave_core::Result<()> {
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
            "creating workspace package symlinks on this platform",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_path_scoped_workspace() {
        let rel =
            relative_path(Path::new("node_modules/@acme/ui"), Path::new("packages/ui")).unwrap();
        assert_eq!(rel, PathBuf::from("../../packages/ui"));
    }

    #[test]
    fn rejects_parent_traversal_targets() {
        assert!(sanitize_workspace_target("../secret").is_err());
        assert!(sanitize_workspace_target("/etc/passwd").is_err());
    }
}
