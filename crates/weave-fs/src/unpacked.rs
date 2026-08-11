//! Unpacked package cache beside the object store.
//!
//! Layout:
//!
//! ```text
//! <store>/../unpacked/sha256/<ab>/<cdef…>/     # extracted package contents
//! <store>/../unpacked/sha256/<ab>/<cdef…>.ready
//! ```

use std::fs::{self, File};
use std::path::{Path, PathBuf};

use weave_core::Error;
use weave_store::{ArtifactId, ContentStore};

use crate::extract::extract_npm_tarball;

/// Content-addressed unpacked package cache.
#[derive(Debug, Clone)]
pub struct UnpackedCache {
    root: PathBuf,
}

impl UnpackedCache {
    /// Open the unpacked cache sibling of a content store's object root.
    pub fn for_store(store: &ContentStore) -> Self {
        let root = store
            .root()
            .parent()
            .map(|p| p.join("unpacked"))
            .unwrap_or_else(|| store.root().join("unpacked"));
        Self { root }
    }

    /// Cache root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn package_dir(&self, id: &ArtifactId) -> PathBuf {
        self.root
            .join("sha256")
            .join(id.shard())
            .join(id.object_name())
    }

    fn ready_marker(&self, id: &ArtifactId) -> PathBuf {
        self.root
            .join("sha256")
            .join(id.shard())
            .join(format!("{}.ready", id.object_name()))
    }

    /// Whether a complete unpacked package exists for `id`.
    pub fn contains(&self, id: &ArtifactId) -> bool {
        self.ready_marker(id).is_file() && self.package_dir(id).is_dir()
    }

    /// Ensure `id` is unpacked; returns `(path, cache_hit)`.
    pub fn ensure(
        &self,
        store: &ContentStore,
        id: &ArtifactId,
    ) -> weave_core::Result<(PathBuf, bool)> {
        let dest = self.package_dir(id);
        if self.contains(id) {
            return Ok((dest, true));
        }

        let shard = self.root.join("sha256").join(id.shard());
        fs::create_dir_all(&shard).map_err(|source| Error::Io {
            path: shard.clone(),
            source,
        })?;

        let tmp = shard.join(format!(".tmp-{}-{}", id.object_name(), std::process::id()));
        if tmp.exists() {
            let _ = clear_tree_writable(&tmp);
            let _ = fs::remove_dir_all(&tmp);
        }
        fs::create_dir_all(&tmp).map_err(|source| Error::Io {
            path: tmp.clone(),
            source,
        })?;

        let bytes = store.get(id)?;
        if let Err(err) = extract_npm_tarball(&bytes, &tmp) {
            let _ = clear_tree_writable(&tmp);
            let _ = fs::remove_dir_all(&tmp);
            return Err(err);
        }
        if let Err(err) = make_tree_readonly(&tmp) {
            let _ = clear_tree_writable(&tmp);
            let _ = fs::remove_dir_all(&tmp);
            return Err(err);
        }

        match fs::rename(&tmp, &dest) {
            Ok(()) => {}
            Err(_) if self.contains(id) => {
                let _ = clear_tree_writable(&tmp);
                let _ = fs::remove_dir_all(&tmp);
                return Ok((self.package_dir(id), true));
            }
            Err(source) => {
                let _ = clear_tree_writable(&tmp);
                let _ = fs::remove_dir_all(&tmp);
                // Dest may exist from a crashed peer without a ready marker.
                if dest.exists() && !self.ready_marker(id).is_file() {
                    let _ = clear_tree_writable(&dest);
                    let _ = fs::remove_dir_all(&dest);
                }
                return Err(Error::Io { path: dest, source });
            }
        }

        let marker = self.ready_marker(id);
        File::create(&marker).map_err(|source| Error::Io {
            path: marker,
            source,
        })?;

        Ok((self.package_dir(id), false))
    }

    /// Remove unpacked package contents and ready marker for `id`, if present.
    pub fn remove(&self, id: &ArtifactId) -> weave_core::Result<()> {
        let dir = self.package_dir(id);
        if dir.is_dir() {
            // Clear readonly bits so remove_dir_all succeeds on Unix.
            let _ = clear_tree_writable(&dir);
            fs::remove_dir_all(&dir).map_err(|source| Error::Io {
                path: dir.clone(),
                source,
            })?;
        }
        let marker = self.ready_marker(id);
        match fs::remove_file(&marker) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(Error::Io {
                    path: marker,
                    source,
                })
            }
        }
        Ok(())
    }

    /// List artifact ids that have a complete unpacked cache entry.
    pub fn list_ids(&self) -> weave_core::Result<Vec<ArtifactId>> {
        let mut out = Vec::new();
        let sha = self.root.join("sha256");
        if !sha.is_dir() {
            return Ok(out);
        }
        for shard in fs::read_dir(&sha).map_err(|source| Error::Io {
            path: sha.clone(),
            source,
        })? {
            let shard = shard.map_err(|source| Error::Io {
                path: sha.clone(),
                source,
            })?;
            let shard_path = shard.path();
            if !shard_path.is_dir() {
                continue;
            }
            let shard_name = shard.file_name().to_string_lossy().into_owned();
            if shard_name.len() != 2 || !shard_name.chars().all(|c| c.is_ascii_hexdigit()) {
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
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.ends_with(".ready") {
                    continue;
                }
                let object = name.trim_end_matches(".ready");
                let hex = format!("{shard_name}{object}");
                if let Ok(id) = ArtifactId::parse(&hex) {
                    if self.contains(&id) {
                        out.push(id);
                    }
                }
            }
        }
        out.sort();
        Ok(out)
    }
}

fn clear_tree_writable(root: &Path) -> weave_core::Result<()> {
    fn walk(path: &Path) -> weave_core::Result<()> {
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
                walk(&entry.path())?;
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = meta.permissions().mode();
            let writable = mode | 0o200;
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(writable));
        }
        Ok(())
    }
    walk(root)
}

fn make_tree_readonly(root: &Path) -> weave_core::Result<()> {
    fn walk(path: &Path) -> weave_core::Result<()> {
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
                walk(&entry.path())?;
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = meta.permissions().mode();
            let readonly = mode & !0o222;
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(readonly));
        }
        Ok(())
    }
    walk(root)
}
