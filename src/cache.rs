//! Persistent cache of normalized provider responses in SQLite.

use std::str::FromStr;
use std::time::Duration;

use jiff::Timestamp;
use serde::{Serialize, de::DeserializeOwned};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

/// Use an in-memory database (tests, throwaway runs).
pub const IN_MEMORY: &str = ":memory:";

#[derive(Clone)]
pub struct ProviderCache {
    pool: SqlitePool,
}

pub struct Cached<T> {
    pub value: T,
    pub fetched_at: Timestamp,
    pub expires_at: Timestamp,
}

impl<T> Cached<T> {
    pub fn is_fresh(&self, now: Timestamp) -> bool {
        now < self.expires_at
    }
}

impl ProviderCache {
    /// Opens (creating if needed) the database at `path` and applies migrations.
    pub async fn open(path: &str) -> Result<Self, sqlx::Error> {
        let pool = if path == IN_MEMORY {
            // Every in-memory connection is a separate database, so keep exactly one.
            SqlitePoolOptions::new()
                .max_connections(1)
                .idle_timeout(None)
                .max_lifetime(None)
                .connect_with(SqliteConnectOptions::from_str("sqlite::memory:")?)
                .await?
        } else {
            SqlitePoolOptions::new()
                .connect_with(
                    SqliteConnectOptions::new()
                        .filename(path)
                        .create_if_missing(true)
                        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                        .busy_timeout(Duration::from_secs(5)),
                )
                .await?
        };
        sqlx::migrate!().run(&pool).await?;
        Ok(Self { pool })
    }

    /// The underlying database, for other tables kept alongside the cache.
    pub fn pool(&self) -> SqlitePool {
        self.pool.clone()
    }

    /// The cached value regardless of expiry; callers decide what stale data is good for.
    pub async fn get<T: DeserializeOwned>(
        &self,
        provider: &str,
        key: &str,
    ) -> Result<Option<Cached<T>>, CacheError> {
        let row: Option<(String, i64, i64)> = sqlx::query_as(
            "SELECT payload, fetched_at, expires_at FROM provider_cache WHERE provider = ? AND key = ?",
        )
        .bind(provider)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
        let Some((payload, fetched_at, expires_at)) = row else {
            return Ok(None);
        };
        Ok(Some(Cached {
            value: serde_json::from_str(&payload)?,
            fetched_at: Timestamp::from_second(fetched_at)?,
            expires_at: Timestamp::from_second(expires_at)?,
        }))
    }

    pub async fn put<T: Serialize>(
        &self,
        provider: &str,
        key: &str,
        value: &T,
        fetched_at: Timestamp,
        ttl: Duration,
    ) -> Result<(), CacheError> {
        let expires_at = fetched_at.as_second().saturating_add(ttl.as_secs() as i64);
        sqlx::query(
            "INSERT INTO provider_cache (provider, key, payload, fetched_at, expires_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT (provider, key) DO UPDATE SET
                 payload = excluded.payload,
                 fetched_at = excluded.fetched_at,
                 expires_at = excluded.expires_at",
        )
        .bind(provider)
        .bind(key)
        .bind(serde_json::to_string(value)?)
        .bind(fetched_at.as_second())
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("cache database: {0}")]
    Database(#[from] sqlx::Error),
    #[error("cache payload: {0}")]
    Payload(#[from] serde_json::Error),
    #[error("cache timestamp: {0}")]
    Timestamp(#[from] jiff::Error),
}
