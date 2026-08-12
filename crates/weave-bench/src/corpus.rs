//! Real-world lockfile corpus loader.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::analyze::{analyze_lockfile, GraphStats};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub id: String,
    pub category: String,
    pub source: ProvenanceSource,
    pub lockfile_sha256: String,
    pub lockfile_bytes: u64,
    pub packages_map_entries: Option<usize>,
    pub lockfile_version: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceSource {
    pub host: String,
    pub repository: String,
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub path: String,
    pub raw_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CorpusEntry {
    pub id: String,
    pub category: String,
    pub dir: PathBuf,
    pub lockfile: PathBuf,
    pub provenance: Option<Provenance>,
    pub stats: Option<GraphStats>,
    pub analyze_error: Option<String>,
}

/// Discover corpus entries under `benchmarks/corpus`.
pub fn load_corpus(root: &Path) -> anyhow::Result<Vec<CorpusEntry>> {
    let mut out = Vec::new();
    if !root.is_dir() {
        anyhow::bail!("corpus root missing: {}", root.display());
    }
    for category in fs::read_dir(root)? {
        let category = category?;
        if !category.file_type()?.is_dir() {
            continue;
        }
        let cat_name = category.file_name().to_string_lossy().into_owned();
        if cat_name == "." || cat_name.starts_with('.') {
            continue;
        }
        // Skip manifest file sitting in root
        for project in fs::read_dir(category.path())? {
            let project = project?;
            if !project.file_type()?.is_dir() {
                continue;
            }
            let dir = project.path();
            let lockfile = dir.join("package-lock.json");
            if !lockfile.is_file() {
                continue;
            }
            let id = project.file_name().to_string_lossy().into_owned();
            let provenance = read_provenance(&dir.join("PROVENANCE.json"));
            let (stats, analyze_error) = match analyze_lockfile(&lockfile) {
                Ok(s) => (Some(s), None),
                Err(e) => (None, Some(e.to_string())),
            };
            out.push(CorpusEntry {
                id,
                category: cat_name.clone(),
                dir,
                lockfile,
                provenance,
                stats,
                analyze_error,
            });
        }
    }
    out.sort_by(|a, b| (&a.category, &a.id).cmp(&(&b.category, &b.id)));
    Ok(out)
}

fn read_provenance(path: &Path) -> Option<Provenance> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Default corpus path relative to cwd / workspace.
pub fn default_corpus_root() -> PathBuf {
    let candidates = [
        PathBuf::from("benchmarks/corpus"),
        PathBuf::from("../benchmarks/corpus"),
        PathBuf::from("../../benchmarks/corpus"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/corpus"),
    ];
    for c in candidates {
        if c.join("MANIFEST.json").is_file() {
            return c;
        }
    }
    PathBuf::from("benchmarks/corpus")
}
