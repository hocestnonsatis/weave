//! JSON report types.

use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::measure::{HostInfo, ScenarioResult};

/// Full suite output.
#[derive(Debug, Clone, Serialize)]
pub struct BenchSuiteResult {
    pub suite: String,
    pub host: HostInfo,
    pub work_dir: String,
    pub rows: Vec<ScenarioResult>,
    pub summary: Option<String>,
}

pub fn write_json(path: &Path, result: &BenchSuiteResult) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_vec_pretty(result).expect("serialize bench result");
    fs::write(path, body)
}
