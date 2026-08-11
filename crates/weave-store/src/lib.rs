//! Content-addressed artifact store for Weave.
//!
//! Logical contract (WEAVE.md §16):
//!
//! ```text
//! contains(id)
//! put(id, bytes)   // atomic
//! open(id) / get(id)
//! remove(id)
//! verify(id)
//! ```
//!
//! Physical layout:
//!
//! ```text
//! <root>/sha256/ab/cdef...   # 64-char lowercase hex digest
//! ```
//!
//! `put` writes to a temporary file in the same directory, fsyncs, then renames
//! into place so a crash cannot leave a valid-looking corrupt object.

#![deny(missing_docs)]

mod id;
mod paths;
mod store;

pub use id::{hash_bytes, ArtifactId};
pub use paths::{default_store_dir, default_weave_home, ensure_store_layout};
pub use store::ContentStore;
