//! Experimental game support contract (§26 of MASTER_SPEC.md).
//!
//! Data and object-safe interfaces only. Core owns policy, paths, processes,
//! transactions and algorithms. A compiled package and a script bridge use the
//! same host operations. There is deliberately no dependency on core or a runtime.
//! Layer descriptions express ownership, not a choice of VFS implementation;
//! write isolation and tracer packages must validate this API before it is fixed.

pub mod game;
pub mod host;
pub mod id;
pub mod layer;
pub mod package;

pub use game::*;
pub use host::*;
pub use id::*;
pub use layer::*;
pub use package::*;

pub const GAME_API_VERSION: semver::Version = semver::Version::new(0, 1, 0);
