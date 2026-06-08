//! Factory: pick the right backend from `StorageConfig`.

use super::sqlite::SqliteStore;
use super::store::{StorageConfig, Store};
use cleanclaw_core::{CleanClawError, Result};

#[cfg(feature = "postgres")]
use super::postgres::PostgresStore;

pub async fn open(cfg: &StorageConfig, home: &std::path::Path) -> Result<Box<dyn Store>> {
    match cfg.r#type {
        super::store::StorageType::Sqlite => {
            let path = if cfg.dsn.is_empty() {
                home.join("cleanclaw.db")
            } else {
                std::path::PathBuf::from(&cfg.dsn)
            };
            // Ensure parent dir exists for non-":memory:" paths.
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            let path_str = path.to_string_lossy().to_string();
            let st = SqliteStore::open(&path_str).await?;
            if cfg.auto_migrate {
                st.migrate().await?;
            }
            Ok(Box::new(st))
        }
        super::store::StorageType::Postgres => {
            // The Postgres backend ships its schema + open + migrate
            // plumbing; the per-method Store-trait impl is a
            // follow-up. Returning NotImplemented at the factory
            // level keeps the SQLite build working while
            // surfacing a clear error to anyone who tries to
            // boot with `StorageType::Postgres`.
            #[cfg(feature = "postgres")]
            {
                let st = PostgresStore::open(&cfg.dsn).await?;
                if cfg.auto_migrate {
                    st.migrate().await?;
                }
                Err(CleanClawError::NotImplemented(
                    "PostgresStore.{} Store-trait impl is a follow-up; \
                     use cleanclaw-store::sqlite::SqliteStore for now"
                        .into(),
                ))
            }
            #[cfg(not(feature = "postgres"))]
            {
                Err(CleanClawError::NotImplemented(
                    "postgres support not compiled in".into(),
                ))
            }
        }
    }
}
