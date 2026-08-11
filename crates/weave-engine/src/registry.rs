//! Global project registry so shared-store GC can find all roots.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use weave_core::Error;
use weave_store::default_weave_home;

use crate::config::ProjectConfig;
use crate::environment::EnvironmentStore;

/// One registered Weave project that may hold environment roots for a store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRegistration {
    /// Absolute project root.
    pub root: String,
    /// Absolute store object path this project uses.
    pub store_path: String,
}

/// Register (or refresh) a project under `$WEAVE_HOME/registry/projects/`.
pub fn register_project(project_root: &Path) -> weave_core::Result<()> {
    let root = canonicalize_or_abs(project_root)?;
    let config = ProjectConfig::load(&root)?;
    let store_path = PathBuf::from(&config.store_path);
    let store_path = canonicalize_or_abs(&store_path)?;
    let home = weave_home_for_store(&store_path)?;
    let dir = home.join("registry").join("projects");
    fs::create_dir_all(&dir).map_err(|source| Error::Io {
        path: dir.clone(),
        source,
    })?;

    let key = registration_key(&root);
    let path = dir.join(format!("{key}.json"));
    let record = ProjectRegistration {
        root: root.display().to_string(),
        store_path: store_path.display().to_string(),
    };
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(&record).map_err(|err| Error::InvalidState {
        path: path.clone(),
        reason: err.to_string(),
    })?;
    fs::write(&tmp, body).map_err(|source| Error::Io {
        path: tmp.clone(),
        source,
    })?;
    fs::rename(&tmp, &path).map_err(|source| Error::Io { path, source })?;
    Ok(())
}

/// Load pins from `$WEAVE_HOME/pins.json` if present.
pub fn load_pins(store_root: &Path) -> weave_core::Result<Vec<String>> {
    let home = weave_home_for_store(store_root)?;
    let path = home.join("pins.json");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let bytes = fs::read(&path).map_err(|source| Error::Io {
        path: path.clone(),
        source,
    })?;
    #[derive(Deserialize)]
    struct PinsFile {
        #[serde(default)]
        artifacts: Vec<String>,
    }
    let file: PinsFile = serde_json::from_slice(&bytes).map_err(|err| Error::InvalidState {
        path,
        reason: format!("invalid pins.json: {err}"),
    })?;
    Ok(file.artifacts)
}

/// Collect absolute project roots registered against `store_root`.
pub fn registered_projects_for_store(store_root: &Path) -> weave_core::Result<Vec<PathBuf>> {
    let store_root = canonicalize_or_abs(store_root)?;
    let home = weave_home_for_store(&store_root)?;
    let dir = home.join("registry").join("projects");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|source| Error::Io {
        path: dir.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| Error::Io {
            path: dir.clone(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        let record: ProjectRegistration =
            serde_json::from_slice(&bytes).map_err(|err| Error::InvalidState {
                path: path.clone(),
                reason: err.to_string(),
            })?;
        let registered_store = canonicalize_or_abs(Path::new(&record.store_path))?;
        if registered_store != store_root {
            continue;
        }
        let project = PathBuf::from(&record.root);
        if project.join(".weave").is_dir() {
            out.push(project);
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// Collect artifact id hex strings from all environments in `project_root`.
pub fn artifact_roots_from_project(project_root: &Path) -> weave_core::Result<Vec<String>> {
    let store = EnvironmentStore::open(project_root);
    let mut roots = Vec::new();
    for env in store.list()? {
        for id in env.artifacts.values() {
            roots.push(id.clone());
        }
    }
    Ok(roots)
}

fn weave_home_for_store(store_root: &Path) -> weave_core::Result<PathBuf> {
    // Prefer WEAVE_HOME / default home when the store lives under …/store/objects.
    if let Ok(home) = default_weave_home() {
        let expected = home.join("store").join("objects");
        if canonicalize_or_abs(&expected).ok().as_ref() == Some(&canonicalize_or_abs(store_root)?) {
            return Ok(home);
        }
        // Store may be a custom path under WEAVE_HOME already.
        if store_root.starts_with(&home) {
            return Ok(home);
        }
    }
    // Fallback: parent of store/objects → weave home.
    if let Some(objects_parent) = store_root.parent() {
        if store_root.file_name().and_then(|s| s.to_str()) == Some("objects") {
            if let Some(store_parent) = objects_parent.parent() {
                if objects_parent.file_name().and_then(|s| s.to_str()) == Some("store") {
                    return Ok(store_parent.to_path_buf());
                }
            }
        }
    }
    store_root
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| Error::InvalidState {
            path: store_root.to_path_buf(),
            reason: "cannot resolve weave home for store".into(),
        })
}

fn registration_key(root: &Path) -> String {
    let digest = Sha256::digest(root.display().to_string().as_bytes());
    hex(&digest[..16])
}

fn canonicalize_or_abs(path: &Path) -> weave_core::Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(p) => Ok(p),
        Err(_) if path.is_absolute() => Ok(path.to_path_buf()),
        Err(source) => Err(Error::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}
