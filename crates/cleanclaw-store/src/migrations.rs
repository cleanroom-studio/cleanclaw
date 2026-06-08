//! Initial schema migrations. Kept as a single SQL string so the same
//! statements can run on SQLite (default) or Postgres.
//!
//! Mirrors the table DDLs in ,
//! stripped to the canonical schema (no pre-DDL renames / ALTERs because
//! this is a fresh-install codebase).

/// SQLite-flavored schema. Uses INTEGER for booleans (sqlx maps them via
/// `bool` derive), TEXT for timestamps in ISO-8601 form.
pub const SCHEMA_SQLITE: &str = include_str!("schema_sqlite.sql");

/// Postgres-flavored schema. Uses BOOLEAN and TIMESTAMPTZ.
pub const SCHEMA_POSTGRES: &str = include_str!("schema_postgres.sql");
