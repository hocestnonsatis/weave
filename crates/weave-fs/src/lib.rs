//! Filesystem materialization primitives for Weave.
//!
//! Packages are extracted once into an unpacked content-addressed cache, then
//! hardlinked (or copied) into candidate trees. Install-script / native
//! packages always copy so shared cache contents stay immutable.

#![deny(missing_docs)]

mod activate;
mod bins;
mod extract;
mod link;
mod materialize;
mod plan;
mod unpacked;
mod workspace;

pub use activate::{activate_candidate, validate_candidate, ActivationReport};
pub use bins::{link_package_bins, BinLinkReport};
pub use extract::{extract_npm_tarball, pack_directory_as_npm_tarball, pack_npm_tarball};
pub use link::{link_or_copy_tree, same_filesystem, LinkStats};
pub use materialize::{materialize_plan, MaterializeReport};
pub use plan::{MaterializationPlan, PlannedPackage};
pub use unpacked::UnpackedCache;
pub use workspace::{wire_workspace_links, WorkspaceLinkReport};

/// Materialization format version (environment identity input).
///
/// Bumped in Phase 5 when optional os/cpu filtering entered the tree shape.
pub fn materialization_version() -> &'static str {
    "4"
}
