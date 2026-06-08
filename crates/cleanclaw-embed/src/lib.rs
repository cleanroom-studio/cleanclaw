//! Embedded web assets. The build pipeline (`make build`) compiles the
//! SvelteKit bundle into `web/build/` which is embedded at compile time
//! via `rust-embed`. If the directory doesn't exist (e.g. during early
//! development), the embed returns no files and the setup server
//! surfaces a 404 with a helpful message.

pub fn web_root_marker() -> &'static str {
    "web/build/"
}
