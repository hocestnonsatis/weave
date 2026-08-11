//! Thin wrapper around the `git` executable.

use std::path::{Path, PathBuf};
use std::process::Command;

use weave_core::Error;

/// Git CLI adapter.
#[derive(Debug, Default, Clone)]
pub struct GitCli;

impl GitCli {
    /// Create a new adapter.
    pub fn new() -> Self {
        Self
    }

    /// Locate the repository root containing `start`, walking parents as Git does.
    pub fn discover_root(&self, start: &Path) -> weave_core::Result<PathBuf> {
        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(start)
            .env("LC_ALL", "C")
            .output()
            .map_err(|source| Error::Git {
                message: format!("failed to spawn git: {source}"),
            })?;

        if !output.status.success() {
            return Err(Error::NotAGitRepository {
                path: start.to_path_buf(),
            });
        }

        let root = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if root.is_empty() {
            return Err(Error::NotAGitRepository {
                path: start.to_path_buf(),
            });
        }
        Ok(PathBuf::from(root))
    }

    /// Return the short HEAD object name.
    pub fn head_short(&self, repo: &Path) -> weave_core::Result<String> {
        let output = self.run(repo, &["rev-parse", "--short", "HEAD"])?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    /// Return the current branch name, or `None` when HEAD is detached.
    pub fn current_branch(&self, repo: &Path) -> weave_core::Result<Option<String>> {
        let output = Command::new("git")
            .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
            .current_dir(repo)
            .env("LC_ALL", "C")
            .output()
            .map_err(|source| Error::Git {
                message: format!("failed to spawn git: {source}"),
            })?;

        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if name.is_empty() {
                Ok(None)
            } else {
                Ok(Some(name))
            }
        } else {
            // Exit code 1 with --quiet means detached HEAD.
            Ok(None)
        }
    }

    /// Return whether the working tree has any uncommitted changes.
    pub fn is_dirty(&self, repo: &Path) -> weave_core::Result<bool> {
        let output = self.run(repo, &["status", "--porcelain"])?;
        Ok(!String::from_utf8_lossy(&output.stdout).trim().is_empty())
    }

    /// Return whether specific paths differ from HEAD (including untracked).
    pub fn paths_dirty(&self, repo: &Path, paths: &[&str]) -> weave_core::Result<bool> {
        if paths.is_empty() {
            return Ok(false);
        }

        let mut args = vec!["status", "--porcelain", "--"];
        args.extend(paths);
        let output = self.run(repo, &args)?;
        Ok(!String::from_utf8_lossy(&output.stdout).trim().is_empty())
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> weave_core::Result<std::process::Output> {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("LC_ALL", "C")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .map_err(|source| Error::Git {
                message: format!("failed to spawn git: {source}"),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let message = if stderr.is_empty() {
                format!("git {} failed", args.join(" "))
            } else {
                stderr
            };
            return Err(Error::Git { message });
        }

        Ok(output)
    }
}
