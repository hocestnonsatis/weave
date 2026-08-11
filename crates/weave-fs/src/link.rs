//! Directory linking primitives (hardlink with copy fallback).

use std::fs;
use std::path::{Path, PathBuf};

use weave_core::Error;

/// Counters for one tree materialization.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LinkStats {
    /// Files placed via hardlink.
    pub hardlinked_files: usize,
    /// Files placed via byte copy.
    pub copied_files: usize,
    /// Directories created.
    pub directories_created: usize,
    /// Symlinks recreated.
    pub symlinks_created: usize,
}

/// Recursively materialize `src` into `dst`.
///
/// When `prefer_hardlink` is true, regular files are hardlinked when possible
/// (same filesystem). Failures fall back to copy. Symlinks are recreated.
/// Directories are created fresh (never hardlinked).
pub fn link_or_copy_tree(
    src: &Path,
    dst: &Path,
    prefer_hardlink: bool,
) -> weave_core::Result<LinkStats> {
    let mut stats = LinkStats::default();
    link_or_copy_tree_inner(src, dst, prefer_hardlink, &mut stats)?;
    Ok(stats)
}

fn link_or_copy_tree_inner(
    src: &Path,
    dst: &Path,
    prefer_hardlink: bool,
    stats: &mut LinkStats,
) -> weave_core::Result<()> {
    let meta = fs::symlink_metadata(src).map_err(|source| Error::Io {
        path: src.to_path_buf(),
        source,
    })?;

    if meta.file_type().is_symlink() {
        let target = fs::read_link(src).map_err(|source| Error::Io {
            path: src.to_path_buf(),
            source,
        })?;
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let _ = fs::remove_file(dst);
            symlink(&target, dst).map_err(|source| Error::Io {
                path: dst.to_path_buf(),
                source,
            })?;
            stats.symlinks_created += 1;
            return Ok(());
        }
        #[cfg(not(unix))]
        {
            let _ = target;
            return Err(Error::NotImplemented(
                "recreating package symlinks on this platform",
            ));
        }
    }

    if meta.is_dir() {
        fs::create_dir_all(dst).map_err(|source| Error::Io {
            path: dst.to_path_buf(),
            source,
        })?;
        stats.directories_created += 1;
        for entry in fs::read_dir(src).map_err(|source| Error::Io {
            path: src.to_path_buf(),
            source,
        })? {
            let entry = entry.map_err(|source| Error::Io {
                path: src.to_path_buf(),
                source,
            })?;
            let name = entry.file_name();
            link_or_copy_tree_inner(&entry.path(), &dst.join(name), prefer_hardlink, stats)?;
        }
        return Ok(());
    }

    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    if prefer_hardlink {
        match fs::hard_link(src, dst) {
            Ok(()) => {
                stats.hardlinked_files += 1;
                return Ok(());
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists || dst.exists() => {
                let _ = fs::remove_file(dst);
                if fs::hard_link(src, dst).is_ok() {
                    stats.hardlinked_files += 1;
                    return Ok(());
                }
            }
            Err(_) => {
                // Cross-device or unsupported — fall through to copy.
            }
        }
    }

    fs::copy(src, dst).map_err(|source| Error::Io {
        path: dst.to_path_buf(),
        source,
    })?;
    stats.copied_files += 1;
    Ok(())
}

/// Return true when `a` and `b` appear to share a filesystem device (Unix).
pub fn same_filesystem(a: &Path, b: &Path) -> bool {
    same_filesystem_inner(a, b)
}

#[cfg(unix)]
fn same_filesystem_inner(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let meta_a = fs::metadata(a).ok();
    let meta_b = fs::metadata(b).ok();
    match (meta_a, meta_b) {
        (Some(ma), Some(mb)) => ma.dev() == mb.dev(),
        _ => {
            // Fall back to parent dirs that exist.
            let pa = existing_ancestor(a);
            let pb = existing_ancestor(b);
            match (fs::metadata(pa).ok(), fs::metadata(pb).ok()) {
                (Some(ma), Some(mb)) => ma.dev() == mb.dev(),
                _ => false,
            }
        }
    }
}

#[cfg(not(unix))]
fn same_filesystem_inner(_a: &Path, _b: &Path) -> bool {
    false
}

fn existing_ancestor(path: &Path) -> PathBuf {
    let mut current = path.to_path_buf();
    loop {
        if current.exists() {
            return current;
        }
        if !current.pop() {
            return PathBuf::from(".");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardlinks_when_preferred() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.txt"), b"hello").unwrap();
        let stats = link_or_copy_tree(&src, &dst, true).unwrap();
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "hello");
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let ino_src = fs::metadata(src.join("a.txt")).unwrap().ino();
            let ino_dst = fs::metadata(dst.join("a.txt")).unwrap().ino();
            assert_eq!(ino_src, ino_dst);
            assert_eq!(stats.hardlinked_files, 1);
            assert_eq!(stats.copied_files, 0);
        }
    }

    #[test]
    fn copies_when_hardlink_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.txt"), b"hello").unwrap();
        let stats = link_or_copy_tree(&src, &dst, false).unwrap();
        assert_eq!(stats.copied_files, 1);
        assert_eq!(stats.hardlinked_files, 0);
    }

    #[test]
    fn cross_filesystem_helper_and_copy_fallback() {
        // Abstraction: when prefer_hardlink is false (cross-device simulation),
        // materialization must copy and still produce a correct tree.
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.txt"), b"cross-fs").unwrap();
        let stats = link_or_copy_tree(&src, &dst, false).unwrap();
        assert_eq!(stats.copied_files, 1);
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "cross-fs");

        // Integration: if /dev/shm is a distinct device, exercise real mismatch.
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let shm = Path::new("/dev/shm");
            if shm.is_dir() {
                let tmp_dev = fs::metadata(tmp.path()).ok().map(|m| m.dev());
                let shm_dev = fs::metadata(shm).ok().map(|m| m.dev());
                if tmp_dev.is_some() && tmp_dev != shm_dev {
                    assert!(!same_filesystem(tmp.path(), shm));
                    let cross_src = shm.join(format!("weave-xfs-{}-src", std::process::id()));
                    let cross_dst = tmp.path().join("cross-dst");
                    let _ = fs::remove_dir_all(&cross_src);
                    fs::create_dir_all(&cross_src).unwrap();
                    fs::write(cross_src.join("x.txt"), b"shm").unwrap();
                    let can = same_filesystem(&cross_src, tmp.path());
                    assert!(!can);
                    let stats = link_or_copy_tree(&cross_src, &cross_dst, can).unwrap();
                    assert!(stats.copied_files >= 1);
                    assert_eq!(stats.hardlinked_files, 0);
                    assert_eq!(fs::read_to_string(cross_dst.join("x.txt")).unwrap(), "shm");
                    let _ = fs::remove_dir_all(&cross_src);
                }
            }
        }
    }
}
