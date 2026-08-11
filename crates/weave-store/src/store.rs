//! Atomic content-addressed store operations.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use weave_core::Error;

use crate::id::{hash_bytes, ArtifactId};
use crate::paths::{ensure_store_layout, object_path, shard_dir};

/// Filesystem-backed content-addressed object store.
#[derive(Debug, Clone)]
pub struct ContentStore {
    root: PathBuf,
}

impl ContentStore {
    /// Open (and create) a store rooted at `root` (`…/objects`).
    pub fn open(root: impl Into<PathBuf>) -> weave_core::Result<Self> {
        let root = root.into();
        ensure_store_layout(&root)?;
        Ok(Self { root })
    }

    /// Store root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Path where `id` would be stored.
    pub fn path_for(&self, id: &ArtifactId) -> PathBuf {
        object_path(&self.root, id)
    }

    /// Whether the store contains a complete object for `id`.
    pub fn contains(&self, id: &ArtifactId) -> bool {
        self.path_for(id).is_file()
    }

    /// Store `bytes` under their content hash.
    ///
    /// If `expected` is `Some`, the content hash must match or the put fails
    /// without writing. If the object already exists and verifies, this is a
    /// no-op success (idempotent reuse).
    pub fn put(
        &self,
        bytes: &[u8],
        expected: Option<&ArtifactId>,
    ) -> weave_core::Result<ArtifactId> {
        let id = hash_bytes(bytes);
        if let Some(expected) = expected {
            if &id != expected {
                return Err(Error::ArtifactHashMismatch {
                    id: expected.to_string(),
                    reason: format!("content hashes to {id}, not {expected}"),
                });
            }
        }

        if self.contains(&id) {
            self.verify(&id)?;
            return Ok(id);
        }

        let dir = shard_dir(&self.root, &id);
        fs::create_dir_all(&dir).map_err(|source| Error::Io {
            path: dir.clone(),
            source,
        })?;

        let final_path = object_path(&self.root, &id);
        let tmp_name = format!(
            ".{}.tmp-{}-{}",
            id.object_name(),
            std::process::id(),
            random_suffix()
        );
        let tmp_path = dir.join(tmp_name);

        // Write + fsync temp object.
        {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp_path)
                .map_err(|source| Error::Io {
                    path: tmp_path.clone(),
                    source,
                })?;
            file.write_all(bytes).map_err(|source| Error::Io {
                path: tmp_path.clone(),
                source,
            })?;
            file.sync_all().map_err(|source| Error::Io {
                path: tmp_path.clone(),
                source,
            })?;
        }

        // Best-effort directory fsync before rename (Linux).
        let _ = fsync_dir(&dir);

        match fs::rename(&tmp_path, &final_path) {
            Ok(()) => {
                let _ = fsync_dir(&dir);
                Ok(id)
            }
            Err(err) => {
                // Concurrent put may have won the race.
                let _ = fs::remove_file(&tmp_path);
                if self.contains(&id) {
                    self.verify(&id)?;
                    Ok(id)
                } else {
                    Err(Error::Io {
                        path: final_path,
                        source: err,
                    })
                }
            }
        }
    }

    /// Read the full object bytes.
    pub fn get(&self, id: &ArtifactId) -> weave_core::Result<Vec<u8>> {
        let path = self.path_for(id);
        let mut file = File::open(&path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                Error::ArtifactNotFound { id: id.to_string() }
            } else {
                Error::Io { path, source }
            }
        })?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).map_err(|source| Error::Io {
            path: self.path_for(id),
            source,
        })?;
        Ok(buf)
    }

    /// Open a read-only handle to the object.
    pub fn open_object(&self, id: &ArtifactId) -> weave_core::Result<File> {
        let path = self.path_for(id);
        File::open(&path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                Error::ArtifactNotFound { id: id.to_string() }
            } else {
                Error::Io { path, source }
            }
        })
    }

    /// Verify that the on-disk bytes hash to `id`.
    pub fn verify(&self, id: &ArtifactId) -> weave_core::Result<()> {
        let bytes = self.get(id)?;
        let actual = hash_bytes(&bytes);
        if &actual != id {
            return Err(Error::CorruptArtifact {
                id: id.to_string(),
                reason: format!("on-disk content hashes to {actual}"),
            });
        }
        Ok(())
    }

    /// Remove an object if present. Missing objects are not an error.
    pub fn remove(&self, id: &ArtifactId) -> weave_core::Result<()> {
        let path = self.path_for(id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(Error::Io { path, source }),
        }
    }

    /// List all complete object ids currently present in the store.
    ///
    /// Temporary put files (`.*.tmp-*`) are ignored.
    pub fn list_ids(&self) -> weave_core::Result<Vec<ArtifactId>> {
        let mut out = Vec::new();
        let objects = self.root.join("sha256");
        if !objects.is_dir() {
            return Ok(out);
        }
        for shard in fs::read_dir(&objects).map_err(|source| Error::Io {
            path: objects.clone(),
            source,
        })? {
            let shard = shard.map_err(|source| Error::Io {
                path: objects.clone(),
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
                let meta = entry.metadata().map_err(|source| Error::Io {
                    path: entry.path(),
                    source,
                })?;
                if !meta.is_file() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                let hex = format!("{shard_name}{name}");
                if let Ok(id) = ArtifactId::parse(&hex) {
                    out.push(id);
                }
            }
        }
        out.sort();
        Ok(out)
    }
}

fn fsync_dir(path: &Path) -> std::io::Result<()> {
    let file = File::open(path)?;
    file.sync_all()
}

fn random_suffix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn put_get_contains_verify_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::open(tmp.path()).unwrap();
        let data = b"immutable artifact bytes";
        let id = store.put(data, None).unwrap();
        assert!(store.contains(&id));
        assert_eq!(store.get(&id).unwrap(), data);
        store.verify(&id).unwrap();
        // Idempotent put
        let id2 = store.put(data, None).unwrap();
        assert_eq!(id, id2);
    }

    #[test]
    fn put_rejects_expected_id_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::open(tmp.path()).unwrap();
        let wrong = hash_bytes(b"other");
        let err = store.put(b"data", Some(&wrong)).unwrap_err();
        assert!(matches!(err, Error::ArtifactHashMismatch { .. }));
        assert!(!store.contains(&wrong));
    }

    #[test]
    fn verify_detects_corruption() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::open(tmp.path()).unwrap();
        let id = store.put(b"good", None).unwrap();
        let path = store.path_for(&id);
        // Corrupt in place (simulating bit rot / incomplete write that somehow landed).
        let mut file = OpenOptions::new().write(true).open(&path).unwrap();
        file.write_all(b"BAD!").unwrap();
        file.sync_all().unwrap();
        let err = store.verify(&id).unwrap_err();
        assert!(matches!(err, Error::CorruptArtifact { .. }));
    }

    #[test]
    fn crashed_temp_files_are_not_visible_objects() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::open(tmp.path()).unwrap();
        let id = hash_bytes(b"never committed");
        let dir = shard_dir(store.root(), &id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(format!(".{}.tmp-dead", id.object_name())),
            b"never committed",
        )
        .unwrap();
        assert!(!store.contains(&id));
        let err = store.get(&id).unwrap_err();
        assert!(matches!(err, Error::ArtifactNotFound { .. }));
    }

    #[test]
    fn concurrent_puts_of_same_bytes_converge() {
        use std::sync::Arc;
        use std::thread;

        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(ContentStore::open(tmp.path()).unwrap());
        let data = b"shared concurrent artifact";
        let mut handles = Vec::new();
        for _ in 0..8 {
            let store = Arc::clone(&store);
            handles.push(thread::spawn(move || store.put(data, None).unwrap()));
        }
        let mut ids = Vec::new();
        for handle in handles {
            ids.push(handle.join().unwrap());
        }
        let first = &ids[0];
        assert!(ids.iter().all(|id| id == first));
        assert_eq!(store.get(first).unwrap(), data);
        store.verify(first).unwrap();
    }

    #[test]
    fn remove_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::open(tmp.path()).unwrap();
        let id = store.put(b"x", None).unwrap();
        store.remove(&id).unwrap();
        store.remove(&id).unwrap();
        assert!(!store.contains(&id));
    }

    #[test]
    fn list_ids_skips_temps_and_survives_concurrent_put() {
        use std::sync::Arc;
        use std::thread;

        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(ContentStore::open(tmp.path()).unwrap());
        let id = store.put(b"listed", None).unwrap();
        let shard = store.root().join("sha256").join(id.shard());
        fs::write(
            shard.join(format!(".{}.tmp-junk", id.object_name())),
            b"nope",
        )
        .unwrap();

        let store2 = Arc::clone(&store);
        let writer = thread::spawn(move || {
            for i in 0..20u8 {
                store2.put(&[b'c', i], None).unwrap();
            }
        });
        let listed = store.list_ids().unwrap();
        assert!(listed.iter().any(|x| x == &id));
        writer.join().unwrap();
        assert!(store.list_ids().unwrap().len() >= 2);
    }
}
