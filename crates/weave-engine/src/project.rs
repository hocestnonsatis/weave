//! Project discovery (Git + Node layout).

use std::path::Path;

use weave_core::{Error, ProjectDiscovery, ProjectLayout, WEAVE_DIR};
use weave_git::GitRepository;
use weave_lockfile::detect_lockfile;

/// Discover a Weave-capable project starting from `start`.
///
/// Requires a Git repository and `package.json`. A lockfile is optional for
/// discovery/status but required for `weave init`.
pub fn discover_project(start: &Path) -> weave_core::Result<ProjectDiscovery> {
    let repo = GitRepository::discover(start)?;
    let root = repo.root;

    let package_json = root.join("package.json");
    if !package_json.is_file() {
        return Err(Error::MissingPackageJson { root: root.clone() });
    }

    let lockfile = detect_lockfile(&root)?;
    let weave_initialized = root.join(WEAVE_DIR).join("config.toml").is_file();

    Ok(ProjectDiscovery {
        layout: ProjectLayout {
            root,
            package_json,
            lockfile: lockfile.as_ref().map(|info| info.path.clone()),
            lockfile_kind: lockfile.map(|info| info.kind),
            weave_initialized,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn init_node_repo(dir: &Path) {
        let status = Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success());
        let _ = Command::new("git")
            .args(["config", "user.email", "weave@example.com"])
            .current_dir(dir)
            .status();
        let _ = Command::new("git")
            .args(["config", "user.name", "Weave Test"])
            .current_dir(dir)
            .status();
        fs::write(
            dir.join("package.json"),
            r#"{"name":"demo","version":"1.0.0"}"#,
        )
        .unwrap();
        fs::write(dir.join("README"), "x").unwrap();
        let status = Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success());
        let status = Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn discovers_node_git_project() {
        let tmp = tempfile::tempdir().unwrap();
        init_node_repo(tmp.path());
        let discovery = discover_project(tmp.path()).unwrap();
        assert!(!discovery.layout.weave_initialized);
        assert!(discovery.layout.lockfile.is_none());
    }
}
