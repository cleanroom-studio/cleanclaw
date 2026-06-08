//! HTTP handlers. Each submodule owns a slice of the dashboard API
//! surface; `Server::router` merges them into the main Router.

pub mod admin;
pub mod agents;
pub mod channels;
pub mod cron;
pub mod extras;
pub mod extras2;
pub mod plugins;
pub mod projects;
pub mod resources;
pub mod scoped;
pub mod skills;
pub mod tools;
pub mod usage;
