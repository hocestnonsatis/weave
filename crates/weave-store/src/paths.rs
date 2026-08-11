//! Store path helpers.

use std::path::{Path, PathBuf};

use weave_core::Error;

use crate::id::ArtifactId;

/// Resolve the default global Weave home (`$HOME/.weave`, overridable via `WEAVE_HOME`).
pub fn default_weave_home() -> weave_core::Result<PathBuf> {
    if let Ok(custom) = std::env::var("WEAVE_HOME") {
        return Ok(PathBuf::from(custom));
    }

    let home = std::env::var_os("HOME").ok_or_else(|| Error::InvalidState {
        path: PathBuf::from("~/.weave"),
        reason: "HOME is not set; set WEAVE_HOME or HOME".into(),
    })?;
    Ok(PathBuf::from(home).join(".weave"))
}

/// Resolve the default content-addressed object store root.
pub fn default_store_dir() -> weave_core::Result<PathBuf> {
    Ok(default_weave_home()?.join("store").join("objects"))
}

/// Ensure the global store directory hierarchy exists.
pub fn ensure_store_layout(store_root: &Path) -> weave_core::Result<()> {
    std::fs::create_dir_all(store_root.join("sha256")).map_err(|source| Error::Io {
        path: store_root.to_path_buf(),
        source,
    })
}

pub(crate) fn object_path(root: &Path, id: &ArtifactId) -> PathBuf {
    root.join("sha256").join(id.shard()).join(id.object_name())
}

pub(crate) fn shard_dir(root: &Path, id: &ArtifactId) -> PathBuf {
    root.join("sha256").join(id.shard())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn creates_store_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("objects");
        ensure_store_layout(&root).unwrap();
        assert!(root.join("sha256").is_dir());
    }

    #[test]
    fn respects_weave_home() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("WEAVE_HOME", tmp.path());
        let store = default_store_dir().unwrap();
        assert_eq!(store, tmp.path().join("store").join("objects"));
        std::env::remove_var("WEAVE_HOME");
    }
}
