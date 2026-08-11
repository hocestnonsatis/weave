//! High-level Git repository snapshot used by Weave.

use std::path::{Path, PathBuf};

use weave_core::Error;

use crate::cli::GitCli;

/// Snapshot of Git repository identity and working-tree state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRepository {
    /// Absolute repository root.
    pub root: PathBuf,
    /// Current branch, if not detached.
    pub branch: Option<String>,
    /// Short HEAD hash.
    pub head: String,
    /// Working tree dirtiness.
    pub working_tree: WorkingTreeState,
}

/// Working-tree dirtiness flags Weave cares about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingTreeState {
    /// Any uncommitted change.
    pub dirty: bool,
    /// `package.json` and/or lockfile differ from HEAD or are untracked.
    pub dependency_files_dirty: bool,
}

impl GitRepository {
    /// Discover and snapshot the repository containing `start`.
    pub fn discover(start: &Path) -> weave_core::Result<Self> {
        let git = GitCli::new();
        let root = git.discover_root(start)?;
        Self::inspect(&root)
    }

    /// Inspect an already-known repository root.
    pub fn inspect(root: &Path) -> weave_core::Result<Self> {
        if !root.is_dir() {
            return Err(Error::NotAGitRepository {
                path: root.to_path_buf(),
            });
        }

        let git = GitCli::new();
        let branch = git.current_branch(root)?;
        let head = git.head_short(root)?;
        let dirty = git.is_dirty(root)?;
        let dependency_files_dirty =
            git.paths_dirty(root, &["package.json", "package-lock.json"])?;

        Ok(Self {
            root: root.to_path_buf(),
            branch,
            head,
            working_tree: WorkingTreeState {
                dirty,
                dependency_files_dirty,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn init_repo(dir: &Path) {
        let status = Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(dir)
            .status()
            .expect("git init");
        assert!(status.success());
        let _ = Command::new("git")
            .args(["config", "user.email", "weave@example.com"])
            .current_dir(dir)
            .status();
        let _ = Command::new("git")
            .args(["config", "user.name", "Weave Test"])
            .current_dir(dir)
            .status();
    }

    #[test]
    fn discovers_clean_repository() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("README"), "hi").unwrap();
        let status = Command::new("git")
            .args(["add", "README"])
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(status.success());
        let status = Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(status.success());

        let repo = GitRepository::discover(tmp.path()).unwrap();
        assert_eq!(repo.branch.as_deref(), Some("main"));
        assert!(!repo.working_tree.dirty);
        assert!(!repo.working_tree.dependency_files_dirty);
        assert!(!repo.head.is_empty());
    }

    #[test]
    fn detects_dirty_dependency_files() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("README"), "hi").unwrap();
        let status = Command::new("git")
            .args(["add", "README"])
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(status.success());
        let status = Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(status.success());

        fs::write(tmp.path().join("package.json"), r#"{"name":"x"}"#).unwrap();

        let repo = GitRepository::discover(tmp.path()).unwrap();
        assert!(repo.working_tree.dirty);
        assert!(repo.working_tree.dependency_files_dirty);
    }

    #[test]
    fn rejects_non_git_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let err = GitRepository::discover(tmp.path()).unwrap_err();
        assert!(matches!(err, Error::NotAGitRepository { .. }));
    }
}
