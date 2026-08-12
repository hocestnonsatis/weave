//! Domain models and shared error types for Weave.
//!
//! This crate defines the core vocabulary of the system. Infrastructure
//! crates (`weave-git`, `weave-store`, …) implement adapters against these
//! types. The CLI must not invent its own domain types.

#![deny(missing_docs)]

mod error;
mod graph;
mod integrity;
mod paths;
mod platform;
mod project;
mod status;

pub use error::{Error, Result};
pub use graph::{
    resolve_dependency_key, DependencyEdge, DependencyGraph, DependencyRef, EdgeKind,
    GraphIdentity, PackageKey, PackageNode, PackageSource, PeerAuditFinding, PeerAuditStatus,
    PeerMeta,
};
pub use integrity::{Integrity, IntegrityAlgo};
pub use paths::{
    WEAVE_BACKUP_NODE_MODULES, WEAVE_CANDIDATE_DIR, WEAVE_CONFIG, WEAVE_DIR,
    WEAVE_ENVIRONMENTS_DIR, WEAVE_METADATA_DIR,
};
pub use platform::{matches_constraint, platform_fit, HostPlatform, PlatformFit};
pub use project::{LockfileKind, ProjectDiscovery, ProjectLayout};
pub use status::{
    DependencyStatus, EnvironmentStatus, EnvironmentSummary, GitStatus, MaterializationStatus,
    ProjectStatus,
};
