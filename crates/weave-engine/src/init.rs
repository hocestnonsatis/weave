//! `weave init` implementation.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use weave_core::{Error, WEAVE_CONFIG, WEAVE_DIR, WEAVE_ENVIRONMENTS_DIR, WEAVE_METADATA_DIR};
use weave_store::{default_store_dir, ensure_store_layout};

use crate::config::ProjectConfig;
use crate::project::discover_project;

/// Result of a successful `weave init` (idempotent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InitOutcome {
    /// Project root that was initialized.
    pub root: PathBuf,
    /// Path to `.weave/`.
    pub weave_dir: PathBuf,
    /// Path to the global object store.
    pub store_dir: PathBuf,
    /// Whether a supported lockfile was present.
    pub lockfile_present: bool,
    /// True when this call created `.weave/` for the first time.
    pub created: bool,
    /// Suggested next CLI commands (agent-friendly).
    pub next_steps: Vec<String>,
}

/// Initialize Weave metadata for the project containing `start`.
///
/// Idempotent: if already initialized, returns success with `created = false`
/// and does not rewrite `config.toml` (preserves reviewed execution policy).
///
/// Does not modify `package.json` or `package-lock.json`.
pub fn init_project(start: &Path) -> weave_core::Result<InitOutcome> {
    let discovery = discover_project(start)?;
    let root = discovery.layout.root;

    // Require a supported lockfile for init so we do not invent dependency state.
    if discovery.layout.lockfile.is_none() {
        return Err(Error::MissingLockfile { root: root.clone() });
    }

    let store_dir = default_store_dir()?;
    ensure_store_layout(&store_dir)?;

    if discovery.layout.weave_initialized {
        let weave_dir = root.join(WEAVE_DIR);
        let _ = crate::registry::register_project(&root);
        ensure_gitignore_entry(&root)?;
        return Ok(InitOutcome {
            root,
            weave_dir,
            store_dir,
            lockfile_present: true,
            created: false,
            next_steps: vec![
                "weave doctor --json".into(),
                "weave switch --json".into(),
                "weave status --json".into(),
            ],
        });
    }

    let weave_dir = root.join(WEAVE_DIR);
    create_dir(&weave_dir)?;
    create_dir(&weave_dir.join(WEAVE_ENVIRONMENTS_DIR))?;
    create_dir(&weave_dir.join(WEAVE_METADATA_DIR))?;

    let config = ProjectConfig::new(store_dir.display().to_string());
    let config_path = weave_dir.join(WEAVE_CONFIG);
    write_config_atomic(&config_path, &config)?;

    ensure_gitignore_entry(&root)?;

    // Best-effort shared-store GC roots registry.
    let _ = crate::registry::register_project(&root);

    Ok(InitOutcome {
        root,
        weave_dir,
        store_dir,
        lockfile_present: true,
        created: true,
        next_steps: vec![
            "weave doctor --json".into(),
            "weave switch --json".into(),
            "weave status --json".into(),
        ],
    })
}

fn create_dir(path: &Path) -> weave_core::Result<()> {
    fs::create_dir_all(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn write_config_atomic(path: &Path, config: &ProjectConfig) -> weave_core::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp_path = parent.join(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("config.toml")
    ));

    let body = toml::to_string_pretty(config).map_err(|err| Error::InvalidState {
        path: path.to_path_buf(),
        reason: format!("failed to serialize config: {err}"),
    })?;

    {
        let mut file = fs::File::create(&tmp_path).map_err(|source| Error::Io {
            path: tmp_path.clone(),
            source,
        })?;
        file.write_all(body.as_bytes())
            .map_err(|source| Error::Io {
                path: tmp_path.clone(),
                source,
            })?;
        file.sync_all().map_err(|source| Error::Io {
            path: tmp_path.clone(),
            source,
        })?;
    }

    fs::rename(&tmp_path, path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Ensure `.gitignore` contains `.weave/` without rewriting unrelated content.
fn ensure_gitignore_entry(root: &Path) -> weave_core::Result<()> {
    let gitignore = root.join(".gitignore");
    let entry = ".weave/";

    if gitignore.is_file() {
        let contents = fs::read_to_string(&gitignore).map_err(|source| Error::Io {
            path: gitignore.clone(),
            source,
        })?;
        let already = contents.lines().any(|line| {
            let trimmed = line.trim();
            trimmed == ".weave/" || trimmed == ".weave" || trimmed == "**/.weave/"
        });
        if already {
            return Ok(());
        }
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&gitignore)
            .map_err(|source| Error::Io {
                path: gitignore.clone(),
                source,
            })?;
        if !contents.is_empty() && !contents.ends_with('\n') {
            writeln!(file).map_err(|source| Error::Io {
                path: gitignore.clone(),
                source,
            })?;
        }
        writeln!(file, "{entry}").map_err(|source| Error::Io {
            path: gitignore.clone(),
            source,
        })?;
    } else {
        fs::write(&gitignore, format!("{entry}\n")).map_err(|source| Error::Io {
            path: gitignore.clone(),
            source,
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::lock_weave_home;
    use std::process::Command;

    fn setup_project(dir: &Path, with_lockfile: bool) {
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
        if with_lockfile {
            fs::write(
                dir.join("package-lock.json"),
                r#"{"name":"demo","lockfileVersion":3,"requires":true,"packages":{}}"#,
            )
            .unwrap();
        }
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
    fn init_creates_weave_metadata_and_is_idempotent() {
        let _guard = lock_weave_home();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
        let project = tmp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        setup_project(&project, true);

        let outcome = init_project(&project).unwrap();
        assert!(outcome.created);
        assert!(outcome.weave_dir.join("config.toml").is_file());
        assert!(outcome.weave_dir.join("environments").is_dir());
        assert!(outcome.weave_dir.join("metadata").is_dir());
        assert!(outcome.store_dir.join("sha256").is_dir());

        let gitignore = fs::read_to_string(project.join(".gitignore")).unwrap();
        assert!(gitignore.contains(".weave/"));

        let again = init_project(&project).unwrap();
        assert!(!again.created);
        assert_eq!(again.root, outcome.root);
        std::env::remove_var("WEAVE_HOME");
    }

    #[test]
    fn init_requires_lockfile() {
        let _guard = lock_weave_home();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("WEAVE_HOME", tmp.path().join("weave-home"));
        let project = tmp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        setup_project(&project, false);

        let err = init_project(&project).unwrap_err();
        assert!(matches!(err, Error::MissingLockfile { .. }));
        std::env::remove_var("WEAVE_HOME");
    }
}
