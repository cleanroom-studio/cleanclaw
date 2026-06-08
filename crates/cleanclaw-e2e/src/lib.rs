//! Stub library so `cargo test -p cleanclaw-e2e` builds the test
//! target. The real smoke tests live in `tests/integration.rs`.

#![allow(clippy::useless_conversion)]

#[allow(dead_code)]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
