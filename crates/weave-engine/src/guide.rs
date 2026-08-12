//! Minimal agent/human adoption guide (CLI-local; no architecture required).

use std::path::Path;

use serde::Serialize;

use crate::project::discover_project;
use crate::status::project_status;

/// Structured adoption guide for agents and humans.
#[derive(Debug, Clone, Serialize)]
pub struct AdoptionGuide {
    /// Short title.
    pub title: String,
    /// Non-negotiable rules (security / semantics).
    pub rules: Vec<String>,
    /// Happy-path command recipe.
    pub recipe: Vec<String>,
    /// Branch-change recipe after Weave is adopted.
    pub after_git_checkout: Vec<String>,
    /// Cleanup recipe.
    pub cleanup: Vec<String>,
    /// Project-specific next steps when run inside a repo (may be empty).
    pub project_next_steps: Vec<String>,
    /// Unsupported / refuse cases.
    pub will_not: Vec<String>,
}

/// Build the static guide, optionally enriched with live project next steps.
pub fn adoption_guide(start: Option<&Path>) -> AdoptionGuide {
    let mut project_next_steps = Vec::new();
    if let Some(start) = start {
        if let Ok(status) = project_status(start) {
            project_next_steps = status.next_steps.clone();
        } else if let Ok(discovery) = discover_project(start) {
            if !discovery.layout.weave_initialized {
                project_next_steps.push("weave init --json".into());
            }
        }
    }

    AdoptionGuide {
        title: "Weave agent/human quickstart".into(),
        rules: vec![
            "Weave materializes node_modules from package-lock.json; it does not replace npm."
                .into(),
            "Never edit package.json / package-lock.json via Weave.".into(),
            "Plain `weave switch` never runs install scripts and never opens script network."
                .into(),
            "Do not set execution.enabled or use --with-exec unless a human reviewed policy."
                .into(),
            "Pass --owner only when you manage agent sessions; Weave never auto-detects agents."
                .into(),
            "Prefer --json on every command you parse.".into(),
        ],
        recipe: vec![
            "weave guide --json          # this document as JSON".into(),
            "weave init --json           # idempotent; creates .weave/ only".into(),
            "weave doctor --json         # fail closed on unsupported / blocked projects".into(),
            "weave switch --json         # materialize + activate (no scripts)".into(),
            "weave status --json         # confirm active env + next_steps".into(),
        ],
        after_git_checkout: vec![
            "git checkout <branch>       # Weave does not run git for you".into(),
            "weave switch --json         # rebuild env for the new lockfile".into(),
            "weave status --json".into(),
        ],
        cleanup: vec![
            "weave recover --json        # clear leftover candidate / dangling active".into(),
            "weave env prune --owner <id> --json   # optional agent metadata cleanup".into(),
            "weave gc --json             # reclaim unreachable CAS artifacts".into(),
        ],
        project_next_steps,
        will_not: vec![
            "Silently replace npm/pnpm/yarn".into(),
            "Auto-enable execution or networking".into(),
            "Auto-detect or trust AI agents".into(),
            "Convert Yarn/pnpm/Bun lockfiles".into(),
            "Invent SRI, URLs, or declared outputs".into(),
        ],
    }
}

/// Human-readable rendering of [`AdoptionGuide`].
pub fn render_adoption_guide(guide: &AdoptionGuide) -> String {
    let mut o = String::new();
    o.push_str(&format!("# {}\n\n", guide.title));
    o.push_str("## Rules\n");
    for r in &guide.rules {
        o.push_str(&format!("- {r}\n"));
    }
    o.push_str("\n## Adopt (existing npm repo)\n");
    for c in &guide.recipe {
        o.push_str(&format!("  {c}\n"));
    }
    o.push_str("\n## After `git checkout`\n");
    for c in &guide.after_git_checkout {
        o.push_str(&format!("  {c}\n"));
    }
    o.push_str("\n## Cleanup / recovery\n");
    for c in &guide.cleanup {
        o.push_str(&format!("  {c}\n"));
    }
    if !guide.project_next_steps.is_empty() {
        o.push_str("\n## This project\n");
        for c in &guide.project_next_steps {
            o.push_str(&format!("  {c}\n"));
        }
    }
    o.push_str("\n## Weave will NOT\n");
    for c in &guide.will_not {
        o.push_str(&format!("- {c}\n"));
    }
    o.push('\n');
    o
}
