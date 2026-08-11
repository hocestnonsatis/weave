//! Safe extraction of npm package tarballs (`*.tgz`).

use std::fs::{self, File};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use tar::Archive;
use weave_core::Error;

/// Extract an npm registry tarball into `dest`.
///
/// npm packs wrap contents in a single top-level `package/` directory; that
/// prefix is stripped. Paths containing `..` or absolute components are rejected.
pub fn extract_npm_tarball(bytes: &[u8], dest: &Path) -> weave_core::Result<()> {
    fs::create_dir_all(dest).map_err(|source| Error::Io {
        path: dest.to_path_buf(),
        source,
    })?;

    let decoder = GzDecoder::new(bytes);
    let mut archive = Archive::new(decoder);
    // We validate paths ourselves.
    archive.set_preserve_permissions(true);
    archive.set_overwrite(true);

    for entry in archive
        .entries()
        .map_err(|err| Error::MaterializationFailed {
            path: dest.to_path_buf(),
            reason: format!("invalid tar: {err}"),
        })?
    {
        let mut entry = entry.map_err(|err| Error::MaterializationFailed {
            path: dest.to_path_buf(),
            reason: format!("invalid tar entry: {err}"),
        })?;

        let entry_path = entry.path().map_err(|err| Error::MaterializationFailed {
            path: dest.to_path_buf(),
            reason: format!("invalid tar path: {err}"),
        })?;

        let relative =
            strip_package_prefix(&entry_path).map_err(|reason| Error::MaterializationFailed {
                path: dest.to_path_buf(),
                reason,
            })?;

        if relative.as_os_str().is_empty() {
            continue;
        }

        let out_path = safe_join(dest, &relative)?;
        let kind = entry.header().entry_type();

        if kind.is_dir() {
            fs::create_dir_all(&out_path).map_err(|source| Error::Io {
                path: out_path.clone(),
                source,
            })?;
            continue;
        }

        if kind.is_symlink() {
            // Symlinks inside packages are allowed only if the link text does
            // not escape the package root when resolved naively.
            let target = entry
                .link_name()
                .map_err(|err| Error::MaterializationFailed {
                    path: out_path.clone(),
                    reason: format!("invalid symlink: {err}"),
                })?
                .ok_or_else(|| Error::MaterializationFailed {
                    path: out_path.clone(),
                    reason: "symlink missing target".into(),
                })?;
            validate_symlink_target(&target)?;
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).map_err(|source| Error::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::symlink;
                let _ = fs::remove_file(&out_path);
                symlink(&target, &out_path).map_err(|source| Error::Io {
                    path: out_path.clone(),
                    source,
                })?;
            }
            #[cfg(not(unix))]
            {
                return Err(Error::NotImplemented(
                    "extracting package symlinks on this platform",
                ));
            }
            continue;
        }

        if !(kind.is_file() || kind.is_hard_link()) {
            // Skip uncommon entry types rather than inventing behavior.
            continue;
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let mut outfile = File::create(&out_path).map_err(|source| Error::Io {
            path: out_path.clone(),
            source,
        })?;
        std::io::copy(&mut entry, &mut outfile).map_err(|source| Error::Io {
            path: out_path.clone(),
            source,
        })?;
        outfile.flush().map_err(|source| Error::Io {
            path: out_path.clone(),
            source,
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(mode) = entry.header().mode() {
                let _ = fs::set_permissions(&out_path, fs::Permissions::from_mode(mode));
            }
        }
    }

    Ok(())
}

fn strip_package_prefix(path: &Path) -> Result<PathBuf, String> {
    let mut comps = path.components();
    match comps.next() {
        Some(Component::Normal(first)) if first == "package" => Ok(comps.as_path().to_path_buf()),
        Some(Component::Normal(_)) => {
            // Some unusual tarballs may omit the package/ prefix; keep as-is
            // after safety checks in safe_join.
            Ok(path.to_path_buf())
        }
        Some(_) => Err(format!(
            "refusing unsafe tar path component in {}",
            path.display()
        )),
        None => Ok(PathBuf::new()),
    }
}

fn safe_join(base: &Path, relative: &Path) -> weave_core::Result<PathBuf> {
    let mut out = base.to_path_buf();
    for comp in relative.components() {
        match comp {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::MaterializationFailed {
                    path: relative.to_path_buf(),
                    reason: format!(
                        "path traversal rejected while extracting to {}",
                        base.display()
                    ),
                });
            }
        }
    }
    Ok(out)
}

fn validate_symlink_target(target: &Path) -> weave_core::Result<()> {
    if target.is_absolute() {
        return Err(Error::MaterializationFailed {
            path: target.to_path_buf(),
            reason: "absolute symlink targets are not allowed".into(),
        });
    }
    for comp in target.components() {
        if matches!(
            comp,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err(Error::MaterializationFailed {
                path: target.to_path_buf(),
                reason: "symlink target escapes package root".into(),
            });
        }
    }
    Ok(())
}

/// Build a minimal npm-style `.tgz` (also used by tests).
pub fn pack_npm_tarball(files: &[(&str, &[u8])]) -> Vec<u8> {
    pack_npm_tarball_with_modes(
        &files
            .iter()
            .map(|(p, d)| ((*p).to_owned(), (*d).to_vec(), 0o644))
            .collect::<Vec<_>>(),
    )
}

/// Build an npm-style `.tgz` with explicit Unix modes.
pub fn pack_npm_tarball_with_modes(files: &[(String, Vec<u8>, u32)]) -> Vec<u8> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use tar::Builder;

    let mut encoded = Vec::new();
    {
        let enc = GzEncoder::new(&mut encoded, Compression::default());
        let mut builder = Builder::new(enc);
        for (path, data, mode) in files {
            let name = format!("package/{path}");
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(*mode);
            header.set_cksum();
            builder
                .append_data(&mut header, name, data.as_slice())
                .unwrap();
        }
        builder.finish().unwrap();
    }
    encoded
}

/// Snapshot a local package directory into an immutable npm-style tarball.
///
/// Walks `dir` deterministically (sorted paths), skips `node_modules`, `.git`,
/// and `.weave`, and rejects paths that escape `dir`. This is the Phase 4
/// `file:` dependency model: **immutable snapshot at acquire time**.
pub fn pack_directory_as_npm_tarball(dir: &Path) -> weave_core::Result<Vec<u8>> {
    let dir = fs::canonicalize(dir).map_err(|source| Error::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    let mut files: Vec<(String, Vec<u8>, u32)> = Vec::new();
    collect_pack_files(&dir, &dir, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(pack_npm_tarball_with_modes(&files))
}

fn collect_pack_files(
    root: &Path,
    current: &Path,
    out: &mut Vec<(String, Vec<u8>, u32)>,
) -> weave_core::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(current)
        .map_err(|source| Error::Io {
            path: current.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| Error::Io {
            path: current.to_path_buf(),
            source,
        })?;
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == "node_modules" || name_str == ".git" || name_str == ".weave" {
            continue;
        }
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        if meta.file_type().is_symlink() {
            // Skip symlinks in local snapshots for safety/reproducibility.
            continue;
        }
        if meta.is_dir() {
            collect_pack_files(root, &path, out)?;
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map_err(|_| Error::MaterializationFailed {
                path: path.clone(),
                reason: "file escapes package directory during snapshot".into(),
            })?;
        for comp in rel.components() {
            if matches!(
                comp,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            ) {
                return Err(Error::MaterializationFailed {
                    path: path.clone(),
                    reason: "path traversal rejected while snapshotting file dependency".into(),
                });
            }
        }
        let bytes = fs::read(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        let mode = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                meta.permissions().mode() & 0o7777
            }
            #[cfg(not(unix))]
            {
                0o644
            }
        };
        out.push((rel.to_string_lossy().replace('\\', "/"), bytes, mode));
    }
    Ok(())
}

/// Build a minimal npm-style `.tgz` for tests.
#[cfg(test)]
pub fn pack_npm_tarball_for_test(files: &[(&str, &[u8])]) -> Vec<u8> {
    pack_npm_tarball(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_and_strips_package_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let tgz = pack_npm_tarball_for_test(&[
            ("package.json", br#"{"name":"demo","version":"1.0.0"}"#),
            ("index.js", b"module.exports = 1;\n"),
        ]);
        let dest = tmp.path().join("out");
        extract_npm_tarball(&tgz, &dest).unwrap();
        assert!(dest.join("package.json").is_file());
        assert!(dest.join("index.js").is_file());
        assert!(!dest.join("package").exists());
    }

    #[test]
    fn rejects_path_traversal_components() {
        let base = Path::new("/tmp/weave-dest");
        let err = safe_join(base, Path::new("foo/../../etc/passwd")).unwrap_err();
        assert!(matches!(err, Error::MaterializationFailed { .. }));
        let err = validate_symlink_target(Path::new("../outside")).unwrap_err();
        assert!(matches!(err, Error::MaterializationFailed { .. }));
        let err = validate_symlink_target(Path::new("/etc/passwd")).unwrap_err();
        assert!(matches!(err, Error::MaterializationFailed { .. }));
    }

    #[test]
    fn rejects_malicious_tar_traversal_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let tgz = malicious_tarball("package/../../evil.txt", b"pwned");
        let dest = tmp.path().join("out");
        let err = extract_npm_tarball(&tgz, &dest).unwrap_err();
        assert!(matches!(err, Error::MaterializationFailed { .. }));
        assert!(!tmp.path().join("evil.txt").exists());
        assert!(!dest.join("evil.txt").exists());
    }

    #[test]
    fn rejects_absolute_symlink_in_tarball() {
        let tmp = tempfile::tempdir().unwrap();
        let tgz = malicious_symlink_tarball("/etc/passwd");
        let dest = tmp.path().join("out");
        let err = extract_npm_tarball(&tgz, &dest).unwrap_err();
        assert!(matches!(err, Error::MaterializationFailed { .. }));
    }

    #[test]
    fn rejects_symlink_traversal_in_tarball() {
        let tmp = tempfile::tempdir().unwrap();
        let tgz = malicious_symlink_tarball("../outside");
        let dest = tmp.path().join("out");
        let err = extract_npm_tarball(&tgz, &dest).unwrap_err();
        assert!(matches!(err, Error::MaterializationFailed { .. }));
    }

    #[test]
    fn pack_directory_snapshot_is_deterministic() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg = tmp.path().join("pkg");
        fs::create_dir_all(pkg.join("lib")).unwrap();
        fs::write(
            pkg.join("package.json"),
            br#"{"name":"pkg","version":"1.0.0"}"#,
        )
        .unwrap();
        fs::write(pkg.join("lib/index.js"), b"module.exports=1;\n").unwrap();
        let a = pack_directory_as_npm_tarball(&pkg).unwrap();
        let b = pack_directory_as_npm_tarball(&pkg).unwrap();
        assert_eq!(a, b);
        let dest = tmp.path().join("out");
        extract_npm_tarball(&a, &dest).unwrap();
        assert_eq!(
            fs::read_to_string(dest.join("lib/index.js")).unwrap(),
            "module.exports=1;\n"
        );
    }

    fn malicious_tarball(entry_name: &str, data: &[u8]) -> Vec<u8> {
        // Build a gzip+ustar blob without tar::Builder path validation so we
        // can feed traversal names into the extractor.
        let mut header = [0u8; 512];
        let name = entry_name.as_bytes();
        assert!(name.len() < 100);
        header[..name.len()].copy_from_slice(name);
        header[100..107].copy_from_slice(b"0000644");
        header[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
        header[136..147].copy_from_slice(b"00000000000");
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263] = b'0';
        header[264] = b'0';
        header[148..156].copy_from_slice(b"        ");
        let chk: u32 = header.iter().map(|&b| u32::from(b)).sum();
        let chk_s = format!("{chk:06o}\0 ");
        header[148..156].copy_from_slice(chk_s.as_bytes());

        let mut tar = Vec::new();
        tar.extend_from_slice(&header);
        tar.extend_from_slice(data);
        let pad = (512 - (data.len() % 512)) % 512;
        tar.extend(std::iter::repeat_n(0u8, pad));
        tar.extend(std::iter::repeat_n(0u8, 1024));

        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;
        let mut encoded = Vec::new();
        {
            let mut enc = GzEncoder::new(&mut encoded, Compression::default());
            enc.write_all(&tar).unwrap();
            enc.finish().unwrap();
        }
        encoded
    }

    fn malicious_symlink_tarball(target: &str) -> Vec<u8> {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use tar::Builder;
        let mut encoded = Vec::new();
        {
            let enc = GzEncoder::new(&mut encoded, Compression::default());
            let mut builder = Builder::new(enc);
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            header.set_mode(0o777);
            header.set_link_name(target).unwrap();
            header.set_cksum();
            builder
                .append_data(&mut header, "package/link", std::io::empty())
                .unwrap();
            builder.finish().unwrap();
        }
        encoded
    }
}
