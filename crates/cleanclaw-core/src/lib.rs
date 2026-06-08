//! CleanClaw core types: errors, identifier newtypes, build metadata.

pub mod buildinfo;
pub mod error;
pub mod idgen;
pub mod ids;
pub mod time;

pub use buildinfo::{BUILD_COMMIT, BUILD_DATE, BUILD_VERSION};
pub use error::{CleanClawError, Result};
pub use idgen::IdGen;
pub use ids::*;
pub use time::now_utc;
