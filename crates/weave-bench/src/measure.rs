//! Metric collection helpers.

use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

/// One timed scenario result.
#[derive(Debug, Clone, Serialize)]
pub struct ScenarioResult {
    pub name: String,
    pub wall_ms: u128,
    pub disk_bytes: Option<u64>,
    pub file_count: Option<u64>,
    pub approx_inodes: Option<u64>,
    pub note: Option<String>,
}

/// Disk accounting that distinguishes apparent vs unique (hardlink-aware) bytes.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct DiskAccounting {
    pub apparent_bytes: u64,
    pub unique_bytes: u64,
    pub file_count: u64,
    pub approx_inodes: u64,
}

/// Host identity for result files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostInfo {
    pub os: String,
    pub arch: String,
}

impl HostInfo {
    pub fn capture() -> Self {
        Self {
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
        }
    }
}

/// Run `f`, returning elapsed duration.
pub fn time_it<R>(f: impl FnOnce() -> R) -> (R, Duration) {
    let start = Instant::now();
    let out = f();
    (out, start.elapsed())
}

/// Apparent disk usage and entry counts under `root`.
pub fn tree_stats(root: &Path) -> (u64, u64, u64) {
    if !root.exists() {
        return (0, 0, 0);
    }
    let mut bytes = 0u64;
    let mut files = 0u64;
    let mut entries = 0u64;
    for entry in WalkDir::new(root).follow_links(false) {
        let Ok(entry) = entry else {
            continue;
        };
        entries += 1;
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_file() {
            files += 1;
            bytes = bytes.saturating_add(meta.len());
        }
    }
    (bytes, files, entries)
}

/// Sum apparent sizes of several roots.
pub fn trees_stats(roots: &[&Path]) -> (u64, u64, u64) {
    let mut bytes = 0u64;
    let mut files = 0u64;
    let mut entries = 0u64;
    for root in roots {
        let (b, f, e) = tree_stats(root);
        bytes = bytes.saturating_add(b);
        files = files.saturating_add(f);
        entries = entries.saturating_add(e);
    }
    (bytes, files, entries)
}

/// Walk `roots` and compute apparent vs unique bytes using `(dev, ino)` dedup.
///
/// Unique bytes count each inode once — suitable for measuring CAS/hardlink
/// sharing across parallel environments. Does not claim block-level sparseness.
pub fn disk_accounting(roots: &[&Path]) -> DiskAccounting {
    let mut seen: HashSet<(u64, u64)> = HashSet::new();
    let mut apparent = 0u64;
    let mut unique = 0u64;
    let mut files = 0u64;
    let mut entries = 0u64;
    for root in roots {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root).follow_links(false) {
            let Ok(entry) = entry else {
                continue;
            };
            entries += 1;
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            files += 1;
            let len = meta.len();
            apparent = apparent.saturating_add(len);
            let key = (meta.dev(), meta.ino());
            if seen.insert(key) {
                unique = unique.saturating_add(len);
            }
        }
    }
    DiskAccounting {
        apparent_bytes: apparent,
        unique_bytes: unique,
        file_count: files,
        approx_inodes: entries,
    }
}

/// Duplicated storage = apparent − unique (bytes counted more than once via hardlinks/copies).
pub fn duplicated_bytes(acc: DiskAccounting) -> u64 {
    acc.apparent_bytes.saturating_sub(acc.unique_bytes)
}
