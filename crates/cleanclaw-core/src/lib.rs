//! CleanClaw core types: errors, identifier newtypes, build metadata.

pub mod error;
pub mod ids;
pub mod buildinfo;
pub mod time;
pub mod idgen;

pub use error::{CleanClawError, Result};
pub use ids::*;
pub use buildinfo::{BUILD_VERSION, BUILD_COMMIT, BUILD_DATE};
pub use time::now_utc;
pub use idgen::IdGen;
