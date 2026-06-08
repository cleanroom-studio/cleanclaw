//! Persistence layer.
//!
//! The `Store` trait is the unified
//! persistence interface; `SqliteStore` is the only implementation we ship
//! today (Postgres skeleton is gated behind the `postgres` feature).

pub mod factory;
pub mod fts;
pub mod migrations;
pub mod models;
pub mod sqlite;

pub mod postgres;

pub mod store;

pub use factory::open;
pub use store::{StorageConfig, StorageType, Store};
